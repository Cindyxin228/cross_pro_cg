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

    // 解析版本要求
    pub fn parse_version_requirement(&self, version_range: &str) -> Result<VersionReq> {
        VersionReq::parse(version_range)
            .map_err(|e| anyhow::anyhow!("解析版本范围失败: {}", e))
    }

    /// 对指定 crate 的所有符合版本做统一 BFS 分析
    pub async fn analyze_all_versions_bfs(
        &self,
        crate_name: &str,
        version_req: &VersionReq,
        target_function_path: &str,
        max_concurrent: usize,
    ) -> Result<()> {
        let versions = self.database.query_crate_versions(crate_name).await?;
        tracing::info!("开始分析 crate: {}，版本范围: {}，共 {} 个版本", crate_name, version_req, versions.len());
        let mut bfs_queue = VecDeque::new();
        for version in versions {
            if version_req.matches(&Version::parse(&version)?) {
                tracing::info!("处理给定 crate {} {} 的 dependents", crate_name, version);
                let dependents = self.database.query_dependents(crate_name).await?;
                for (dep_name, dep_version, req) in dependents.iter() {
                    // patch
                    let dep_krate = Krate::new(dep_name.clone(), dep_version.clone());
                    let dep_dir = dep_krate.get_crate_dir_path().await?;
                    Krate::patch_cargo_toml_with_parent(&dep_dir, crate_name, &version).await?;
                    // 判断是否调用目标函数
                    if self.is_valid_dependent(
                        &version,
                        req,
                        dep_name,
                        dep_version,
                        target_function_path,
                    ).await.unwrap_or(false) {
                        tracing::info!("依赖者 {} {} 满足条件，加入BFS队列", dep_name, dep_version);
                        bfs_queue.push_back(BfsNode {
                            krate: dep_krate,
                            parent: Some((crate_name.to_string(), version.clone())),
                        });
                    } else {
                        tracing::info!("依赖者 {} {} 不满足条件，跳过", dep_name, dep_version);
                    }
                }
            }
        }
        tracing::info!("所有给定 crate 版本的 dependents 处理完毕，BFS队列长度:{}", bfs_queue.len());
        self.bfs_from_queue(bfs_queue, target_function_path, max_concurrent).await?;
        tracing::info!("analyze_all_versions_bfs: 所有版本分析完成");
        Ok(())
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
        tracing::info!("process_bfs_level: 本层节点数:{}，最大并发:{}", current_level.len(), max_concurrent);
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

        let mut next_nodes = Vec::new();
        let mut total_new = 0;
        for result in results {
            if let Ok(nodes) = result {
                total_new += nodes.len();
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
        tracing::info!("process_bfs_level: 本层发现新节点:{}", total_new);
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

        let crate_dir = bfs_node.krate.get_crate_dir_path().await?;
        tracing::info!("节点 {} {} 解压目录: {}", bfs_node.krate.name(), bfs_node.krate.version(), crate_dir.display());
        if let Some((ref parent_name, ref parent_version)) = bfs_node.parent {
            tracing::info!("为节点 {} {} patch 父依赖 {} ={}", bfs_node.krate.name(), bfs_node.krate.version(), parent_name, parent_version);
            Krate::patch_cargo_toml_with_parent(&crate_dir, parent_name, parent_version).await?;
        }

        tracing::info!("分析节点 {} {} 是否调用目标函数", bfs_node.krate.name(), bfs_node.krate.version());
        self.analyze_function_calls(
            &bfs_node.krate.name(),
            &bfs_node.krate.version(),
            target_function_path,
        ).await;

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
                    krate: Krate::new(dep_name.clone(), dep_version.clone()),
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

    async fn bfs_from_queue(
        &self,
        mut queue: VecDeque<BfsNode>,
        target_function_path: &str,
        max_concurrent: usize,
    ) -> Result<()> {
        let mut visited = HashSet::new();
        let mut level = 0;
        while !queue.is_empty() {
            level += 1;
            tracing::info!("BFS第{}层，队列长度:{}", level, queue.len());
            let current_level = self.pop_bfs_level(&mut queue);
            let results = self.process_bfs_level(
                current_level,
                target_function_path,
                max_concurrent,
                &mut visited,
            ).await?;
            self.push_next_level(&mut queue, results, &mut visited);
        }
        Ok(())
    }
}
