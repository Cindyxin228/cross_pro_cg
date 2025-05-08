use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use once_cell::sync::Lazy;
use tokio::sync::Semaphore;

use anyhow::{Result, Context};
use semver::{Version, VersionReq};
use serde_json::Value;
use tokio::fs as tokio_fs;
use tokio::process::Command;
use tracing::{info, warn};
use futures::{stream, StreamExt};

use crate::database::Database;
use crate::krate::Krate;

// 全局限流器，最多允许 N 个分析任务并发
static GLOBAL_ANALYSIS_SEMAPHORE: Lazy<Arc<Semaphore>> = Lazy::new(|| Arc::new(Semaphore::new(32))); // 32 可根据需要调整

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct CrateVersion {
    pub name: String,
    pub version: String,
}

#[derive(Clone)]
struct BfsNode {
    krate: Krate,
    parent: Option<(String, String)>, // (父节点名, 父节点版本)
}

#[derive(Debug, Clone)]
pub struct DependencyAnalyzer {
    database: Arc<Database>,
}

impl DependencyAnalyzer {
    pub async fn new() -> Result<Self> {
        let database = Database::new().await?;
        Ok(Self {
            database: Arc::new(database),
        })
    }

    fn get_original_dir(&self) -> PathBuf {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    // 主函数改名为更具体的名字
    async fn analyze_function_calls(
        &self,
        crate_name: &str,
        crate_version: &str,
        function_path: &str,
    ) -> Option<String> {
        // info!(
        //     "开始分析 crate {} {} 的函数调用: {}",
        //     crate_name, crate_version, function_path
        // );

        let krate = Krate::new(crate_name.to_string(), crate_version.to_string());
        let original_dir = self.get_original_dir();
        
        // 准备分析环境
        let crate_dir = match self.prepare_analysis_environment(&krate, &original_dir).await {
            Ok(dir) => dir,
            Err(e) => {
                // warn!("准备分析环境失败: {}", e);
                return None;
            }
        };

        // 运行函数调用分析工具
        let analysis_result = self.run_function_analysis(&crate_dir, function_path).await;
        
        // 清理环境并返回结果
        let result = self.cleanup_and_return_result(
            &krate,
            &crate_dir,
            &original_dir,
            analysis_result,
        ).await;

        // 如果分析成功且有结果，保存到项目目录
        if let Some(callers_content) = &result {
            if let Err(e) = self.save_analysis_result(crate_name, crate_version, &crate_dir).await {
                warn!("保存分析结果失败: {}", e);
            }
        }

        result
    }

    // 准备分析环境
    async fn prepare_analysis_environment(
        &self,
        krate: &Krate,
        _original_dir: &PathBuf,
    ) -> Result<PathBuf> {
        // info!("准备分析环境: {} {}", krate.name(), krate.version());

        // 下载并解压crate（已自动判断是否已存在）
        let crate_dir = krate.get_crate_dir_path().await.context(format!(
            "无法下载或解压 crate: {} {}",
            krate.name(), krate.version()
        ))?;

        info!("crate目录已就绪: {}", crate_dir.display());
        Ok(crate_dir)
    }

    // 运行函数调用分析工具
    async fn run_function_analysis(
        &self,
        crate_dir: &PathBuf,
        function_path: &str,
    ) -> Result<Option<String>> {
        info!("运行函数调用分析工具，目标函数: {}", function_path);

        let manifest_path = crate_dir.join("Cargo.toml");
        let output_dir = crate_dir.join("target"); // 工具生成在 crate 目录下

        let mut cmd = Command::new("call-cg4rs");
        cmd.args(&[
            "--find-callers", function_path,
            "--json-output",
            "--manifest-path", &manifest_path.to_string_lossy(),
            "--output-dir", &output_dir.to_string_lossy(),
        ]);
        cmd.env("ROOT_PATH", &crate_dir);

        let call_cg_result = cmd
            .output()
            .await
            .context("运行call-cg4rs工具失败")?;

        if !call_cg_result.status.success() {
            let stderr = String::from_utf8_lossy(&call_cg_result.stderr);
            warn!("call-cg4rs工具执行失败: {}", stderr);
            return Ok(None);
        }

        // 工具生成的 callers.json 路径
        let callers_json_path = output_dir.join("callers.json");
        if !callers_json_path.exists() {
            info!("未找到callers.json文件，说明没有函数调用");
            return Ok(None);
        }

        // 读取callers.json内容
        let callers_content = tokio_fs::read_to_string(&callers_json_path)
            .await
            .context(format!("读取callers.json文件失败: {}", callers_json_path.display()))?;

        Ok(Some(callers_content))
    }

    // 保存分析结果到项目目录
    async fn save_analysis_result(
        &self,
        crate_name: &str,
        crate_version: &str,
        crate_dir: &PathBuf,
    ) -> Result<()> {
        let src_path = crate_dir.join("target").join("callers.json");
        let result_filename = format!("{}-{}-callers.json", crate_name, crate_version);
        let dst_path = Path::new("target").join(&result_filename);

        // 确保 target 目录存在
        if let Some(parent) = dst_path.parent() {
            tokio_fs::create_dir_all(parent)
                .await
                .context("创建target目录失败")?;
        }

        // 复制文件
        tokio_fs::copy(&src_path, &dst_path)
            .await
            .context(format!("复制callers.json到目标目录失败: {} -> {}", src_path.display(), dst_path.display()))?;

        info!("已保存结果到: {}", dst_path.display());
        Ok(())
    }

    // 清理环境并返回结果
    async fn cleanup_and_return_result(
        &self,
        krate: &Krate,
        _crate_dir: &PathBuf,
        _original_dir: &PathBuf,
        analysis_result: Result<Option<String>>,
    ) -> Option<String> {
        // 只清理下载的 .crate 压缩包，不删除解压后的项目文件夹
        let _ = krate.cleanup_crate_file().await;

        match analysis_result {
            Ok(Some(result)) => {
                info!(
                    "crate {} {} 调用了目标函数",
                    krate.name(), krate.version()
                );
                Some(result)
            }
            Ok(None) => {
                info!(
                    "crate {} {} 没有调用目标函数",
                    krate.name(), krate.version()
                );
                None
            }
            Err(e) => {
                warn!(
                    "分析 crate {} {} 时发生错误: {}",
                    krate.name(), krate.version(), e
                );
                None
            }
        }
    }

    // 主函数改名为更具体的名字
    pub async fn build_dependency_tree(
        &self,
        start_crate: &str,
        version_range: &str,
        target_function_path: &str,
    ) -> Result<Krate> {
        info!(
            "开始构建依赖树: {} {} 目标函数: {}",
            start_crate, version_range, target_function_path
        );

        let version_req = self.parse_version_requirement(version_range)?;
        let mut root = self.create_root_node();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        // 初始化根节点的直接子节点
        self.initialize_root_children(&mut root, start_crate, &version_req, &mut queue).await?;
        info!("已初始化根节点的直接子节点，共 {} 个", root.dependents().len());

        // 并发BFS遍历构建依赖树
        let max_concurrent = 32; // 可根据机器核数调整
        self.build_tree_bfs_parallel(&mut root, &mut queue, &mut visited, target_function_path, max_concurrent).await?;

        info!("依赖树构建完成，共访问了 {} 个节点", visited.len());
        Ok(root)
    }

    // 解析版本要求
    pub fn parse_version_requirement(&self, version_range: &str) -> Result<VersionReq> {
        VersionReq::parse(version_range)
            .map_err(|e| anyhow::anyhow!("解析版本范围失败: {}", e))
    }

    // 创建根节点
    pub fn create_root_node(&self) -> Krate {
        Krate::new("root".to_string(), "0.0.0".to_string())
    }

    // 初始化根节点的直接子节点
    pub async fn initialize_root_children(
        &self,
        root: &mut Krate,
        start_crate: &str,
        version_req: &VersionReq,
        queue: &mut VecDeque<Krate>,
    ) -> Result<()> {
        let versions = self.database.query_crate_versions(start_crate).await?;
        info!("获取到起始crate {} 的所有版本，共 {} 个", start_crate, versions.len());
        
        for version in versions {
            if let Ok(ver) = Version::parse(&version) {
                if version_req.matches(&ver) {
                    info!("版本 {} 符合要求，添加到依赖树", version);
                    let start_krate = Krate::new(start_crate.to_string(), version);
                    root.dependents_mut().push(start_krate.clone());
                    queue.push_back(start_krate);
                }
            }
        }
        Ok(())
    }

    /// 并发BFS遍历依赖树
    async fn build_tree_bfs_parallel(
        &self,
        root: &mut Krate,
        queue: &mut VecDeque<Krate>,
        visited: &mut HashSet<CrateVersion>,
        target_function_path: &str,
        max_concurrent: usize,
    ) -> Result<()> {
        while !queue.is_empty() {
            // 取出当前层所有节点
            let mut current_level = Vec::new();
            while let Some(node) = queue.pop_front() {
                current_level.push(node);
            }

            // 并发分析当前层所有节点
            let analyzer = Arc::new(self.clone());
            let results = stream::iter(current_level)
                .map(|current_krate| {
                    let analyzer = Arc::clone(&analyzer);
                    let target_function_path = target_function_path.to_string();
                    async move {
                        analyzer.process_dependents_parallel(
                            current_krate,
                            target_function_path,
                        ).await
                    }
                })
                .buffer_unordered(max_concurrent)
                .collect::<Vec<_>>()
                .await;

            // 把新发现的节点加入队列
            for result in results {
                if let Ok(new_nodes) = result {
                    for new_node in new_nodes {
                        let cv = CrateVersion {
                            name: new_node.name().to_string(),
                            version: new_node.version().to_string(),
                        };
                        if visited.insert(cv) {
                            queue.push_back(new_node);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// 并发处理 dependents
    async fn process_dependents_parallel(
        &self,
        current_krate: Krate,
        target_function_path: String,
    ) -> Result<Vec<Krate>> {
        let dependents = self.database.query_dependents(&current_krate.name()).await?;
        let mut new_nodes = Vec::new();

        let futures = dependents.into_iter().map(|(dep_name, dep_version, req)| {
            let analyzer = self.clone();
            let current_version = current_krate.version().to_string();
            let target_function_path = target_function_path.clone();
            async move {
                let _permit = GLOBAL_ANALYSIS_SEMAPHORE.acquire().await.unwrap();

                if analyzer.is_valid_dependent(
                    &current_version,
                    &req,
                    &dep_name,
                    &dep_version,
                    &target_function_path,
                ).await.unwrap_or(false) {
                    Some(Krate::new(dep_name, dep_version))
                } else {
                    None
                }
            }
        });

        let results = futures::future::join_all(futures).await;
        for node in results.into_iter().flatten() {
            new_nodes.push(node);
        }
        Ok(new_nodes)
    }

    // 检查依赖者是否有效（版本匹配且调用了目标函数）
    async fn is_valid_dependent(
        &self,
        current_version: &str,
        req: &str,
        dep_name: &str,
        dep_version: &str,
        target_function_path: &str,
    ) -> Result<bool> {
        if let (Ok(ver), Ok(dep_req)) = (Version::parse(current_version), VersionReq::parse(req)) {
            if dep_req.matches(&ver) {
                let has_function_call = self.analyze_function_calls(dep_name, dep_version, target_function_path)
                    .await
                    .is_some();
                if has_function_call {
                    info!(
                        "依赖者 {} {} 版本匹配且调用了目标函数",
                        dep_name, dep_version
                    );
                } else {
                    info!(
                        "依赖者 {} {} 版本匹配但未调用目标函数",
                        dep_name, dep_version
                    );
                }
                return Ok(has_function_call);
            }
        }
        Ok(false)
    }

    pub async fn bfs_dependency_analysis(
        &self,
        root: &Krate,
        target_function_path: &str,
        max_concurrent: usize,
    ) -> Result<()> {
        tracing::info!("BFS分析启动，目标函数: {}，最大并发: {}", target_function_path, max_concurrent);
        let mut queue = self.init_bfs_queue(root).await?;
        let mut visited = HashSet::new();
        let mut level = 0;

        while !queue.is_empty() {
            level += 1;
            tracing::info!("========== 开始BFS第{}层，队列长度:{} ==========", level, queue.len());
            let current_level = self.pop_bfs_level(&mut queue);
            let results = self.process_bfs_level(current_level, target_function_path, max_concurrent, &mut visited).await?;
            self.push_next_level(&mut queue, results, &mut visited);
            tracing::info!("========== 结束BFS第{}层，剩余队列:{} ==========", level, queue.len());
        }
        tracing::info!("BFS分析完成，总共访问了 {} 个节点", visited.len());
        Ok(())
    }

    async fn init_bfs_queue(&self, root: &Krate) -> Result<VecDeque<BfsNode>> {
        let mut queue = VecDeque::new();
        for child in root.dependents() {
            queue.push_back(BfsNode {
                krate: child.clone(),
                parent: None,
            });
        }
        tracing::info!("BFS初始化队列，根节点有 {} 个直接子节点", queue.len());
        Ok(queue)
    }

    fn pop_bfs_level(&self, queue: &mut VecDeque<BfsNode>) -> Vec<BfsNode> {
        let mut current_level = Vec::new();
        while let Some(node) = queue.pop_front() {
            current_level.push(node);
        }
        tracing::info!("BFS弹出一层，共 {} 个节点", current_level.len());
        current_level
    }

    async fn process_bfs_level(
        &self,
        current_level: Vec<BfsNode>,
        target_function_path: &str,
        max_concurrent: usize,
        visited: &mut HashSet<CrateVersion>,
    ) -> Result<Vec<BfsNode>> {
        let analyzer = Arc::new(self.clone());
        let results = stream::iter(current_level)
            .map(|bfs_node| {
                let analyzer = Arc::clone(&analyzer);
                let target_function_path = target_function_path.to_string();
                async move {
                    analyzer.process_single_bfs_node(bfs_node, &target_function_path).await
                }
            })
            .buffer_unordered(max_concurrent)
            .collect::<Vec<_>>()
            .await;

        // 收集所有下一层节点acquire().await
        let mut next_nodes = Vec::new();
        for result in results {
            if let Ok(nodes) = result {
                for node in nodes {
                    let cv = CrateVersion {
                        name: node.krate.name().to_string(),
                        version: node.krate.version().to_string(),
                    };
                    if visited.insert(cv) {
                        next_nodes.push(node);
                    }
                }
            }
        }
        Ok(next_nodes)
    }

    async fn process_single_bfs_node(
        &self,
        bfs_node: BfsNode,
        target_function_path: &str,
    ) -> Result<Vec<BfsNode>> {
        tracing::info!(
            "处理节点: {} {}，父节点: {:?}",
            bfs_node.krate.name(),
            bfs_node.krate.version(),
            bfs_node.parent
        );

        // 1. 获取 crate 解压目录
        let crate_dir = bfs_node.krate.get_crate_dir_path().await?;
        tracing::info!("节点 {} {} 解压目录: {}", bfs_node.krate.name(), bfs_node.krate.version(), crate_dir.display());

        // 2. patch Cargo.toml
        if let Some((ref parent_name, ref parent_version)) = bfs_node.parent {
            tracing::info!("为节点 {} {} patch 父依赖 {} ={}", bfs_node.krate.name(), bfs_node.krate.version(), parent_name, parent_version);
            Krate::patch_cargo_toml_with_parent(&crate_dir, parent_name, parent_version).await?;
        }

        // 3. 分析
        tracing::info!("分析节点 {} {} 是否调用目标函数", bfs_node.krate.name(), bfs_node.krate.version());
        self.analyze_function_calls(
            &bfs_node.krate.name(),
            &bfs_node.krate.version(),
            target_function_path,
        ).await;

        // 4. 查询 dependents，筛选有效的依赖者
        let dependents = self.database.query_dependents(&bfs_node.krate.name()).await?;
        tracing::info!("节点 {} {} 有 {} 个直接依赖者", bfs_node.krate.name(), bfs_node.krate.version(), dependents.len());
        let mut next_nodes = Vec::new();
        for (dep_name, dep_version, req) in dependents {
            if self.is_valid_dependent(
                &bfs_node.krate.version(),
                &req,
                &dep_name,
                &dep_version,
                target_function_path,
            ).await.unwrap_or(false) {
                tracing::info!("依赖者 {} {} 满足条件，加入下一层", dep_name, dep_version);
                next_nodes.push(BfsNode {
                    krate: Krate::new(dep_name, dep_version),
                    parent: Some((bfs_node.krate.name().to_string(), bfs_node.krate.version().to_string())),
                });
            } else {
                tracing::info!("依赖者 {} {} 不满足条件，跳过", dep_name, dep_version);
            }
        }
        Ok(next_nodes)
    }

    fn push_next_level(
        &self,
        queue: &mut VecDeque<BfsNode>,
        next_nodes: Vec<BfsNode>,
        _visited: &mut HashSet<CrateVersion>,
    ) {
        let count = next_nodes.len();
        for node in next_nodes {
            queue.push_back(node);
        }
        tracing::info!("BFS推入下一层，共 {} 个节点", count);
    }
}
