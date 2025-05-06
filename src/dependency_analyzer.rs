use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, Context};
use semver::{Version, VersionReq};
use serde_json::Value;
use tokio::fs as tokio_fs;
use tokio::process::Command;
use tracing::{info, warn};

use crate::database::Database;
use crate::krate::Krate;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct CrateVersion {
    pub name: String,
    pub version: String,
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
        info!(
            "开始分析 crate {} {} 的函数调用: {}",
            crate_name, crate_version, function_path
        );

        let krate = Krate::new(crate_name.to_string(), crate_version.to_string());
        let original_dir = self.get_original_dir();
        
        // 准备分析环境
        let crate_dir = match self.prepare_analysis_environment(&krate, &original_dir).await {
            Ok(dir) => dir,
            Err(e) => {
                warn!("准备分析环境失败: {}", e);
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
        info!("准备分析环境: {} {}", krate.name(), krate.version());

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

        // BFS遍历构建依赖树
        self.build_tree_bfs(&mut root, &mut queue, &mut visited, target_function_path).await?;

        info!("依赖树构建完成，共访问了 {} 个节点", visited.len());
        Ok(root)
    }

    // 解析版本要求
    fn parse_version_requirement(&self, version_range: &str) -> Result<VersionReq> {
        VersionReq::parse(version_range)
            .map_err(|e| anyhow::anyhow!("解析版本范围失败: {}", e))
    }

    // 创建根节点
    fn create_root_node(&self) -> Krate {
        Krate::new("root".to_string(), "0.0.0".to_string())
    }

    // 初始化根节点的直接子节点
    async fn initialize_root_children(
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

    // BFS遍历构建依赖树
    async fn build_tree_bfs(
        &self,
        root: &mut Krate,
        queue: &mut VecDeque<Krate>,
        visited: &mut HashSet<CrateVersion>,
        target_function_path: &str,
    ) -> Result<()> {
        while let Some(current_krate) = queue.pop_front() {
            info!(
                "处理节点: {} {}, 剩余队列长度: {}",
                current_krate.name(),
                current_krate.version(),
                queue.len()
            );
            self.process_dependents(
                &current_krate,
                queue,
                visited,
                target_function_path,
            ).await?;
        }
        Ok(())
    }

    // 处理当前节点的所有依赖者
    async fn process_dependents(
        &self,
        current_krate: &Krate,
        queue: &mut VecDeque<Krate>,
        visited: &mut HashSet<CrateVersion>,
        target_function_path: &str,
    ) -> Result<()> {
        let dependents = self.database.query_dependents(&current_krate.name()).await?;
        info!(
            "获取到 {} {} 的依赖者，共 {} 个",
            current_krate.name(),
            current_krate.version(),
            dependents.len()
        );

        for (dep_name, dep_version, req) in dependents {
            if self.is_valid_dependent(
                &current_krate.version(),
                &req,
                &dep_name,
                &dep_version,
                target_function_path,
            ).await? {
                info!(
                    "依赖者 {} {} 调用了目标函数 {}",
                    dep_name, dep_version, target_function_path
                );

                let dep_crate_ver = CrateVersion {
                    name: dep_name.clone(),
                    version: dep_version.clone(),
                };
                if !visited.contains(&dep_crate_ver) {
                    // 先 clone 一份用于日志
                    let dep_name_log = dep_name.clone();
                    let dep_version_log = dep_version.clone();
                    let new_krate = Krate::new(dep_name, dep_version);
                    let mut krate = current_krate.clone();
                    krate.dependents_mut().push(new_krate.clone());
                    queue.push_back(new_krate);
                    visited.insert(dep_crate_ver);
                    info!(
                        "将依赖者 {} {} 添加到依赖树",
                        dep_name_log, dep_version_log
                    );
                } else {
                    info!(
                        "依赖者 {} {} 已访问过，跳过",
                        dep_name, dep_version
                    );
                }
            }
        }
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
}
