use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use futures::{stream, StreamExt};
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
pub struct DependencyNode {
    pub name: String,
    pub version: String,
    pub dependents: Vec<(String, String, String)>, // (dependent_name, dependent_version, version_req)
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

    // 检查crate是否调用了指定的函数，并返回callers.txt的内容
    async fn check_function_call(
        &self,
        crate_name: &str,
        crate_version: &str,
        function_path: &str,
    ) -> Option<String> {
        info!(
            "检查 crate {} {} 是否调用了函数 {}",
            crate_name, crate_version, function_path
        );

        let krate = Krate::new(crate_name.to_string(), crate_version.to_string());

        // 保存原始工作目录
        let original_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        info!("原始工作目录: {}", original_dir.display());

        // 下载并解压crate
        let crate_dir = match krate.get_crate_dir_path().await {
            Ok(dir) => dir,
            Err(e) => {
                warn!("无法下载或解压 crate: {} {}", crate_name, crate_version);
                // 返回原始工作目录
                if std::env::set_current_dir(&original_dir).is_err() {
                    warn!("无法返回原始工作目录: {}", original_dir.display());
                }
                return None;
            }
        };

        // 进入crate目录
        if std::env::set_current_dir(&crate_dir).is_err() {
            warn!("无法进入crate目录: {}", crate_dir.display());
            // 返回原始工作目录
            if std::env::set_current_dir(&original_dir).is_err() {
                warn!("无法返回原始工作目录: {}", original_dir.display());
            }
            return None;
        }

        // 检查当前工作目录
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        info!("当前工作目录: {}", current_dir.display());

        // 列出当前目录内容
        if let Ok(mut entries) = tokio_fs::read_dir(".").await {
            info!("当前目录内容:");
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Ok(path) = entry.path().into_os_string().into_string() {
                    info!("  {}", path);
                }
            }
        }

        // 运行call-cg4rs工具
        info!("运行call-cg4rs工具");

        // 检查tmp目录权限
        let tmp_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        info!("当前工作目录: {}", tmp_dir.display());

        let call_cg_result = Command::new("call-cg4rs")
            .args(&["--find-callers", function_path, "--json-output"]) // 移除 --quiet 以查看更多输出
            .output()
            .await;

        if let Err(e) = call_cg_result {
            warn!("运行call-cg4rs工具失败: {}", e);
            // 返回原始工作目录
            if std::env::set_current_dir(&original_dir).is_err() {
                warn!("无法返回原始工作目录: {}", original_dir.display());
            }
            return None;
        }

        if let Ok(output) = call_cg_result {
            info!("call-cg4rs 退出码: {}", output.status);

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("call-cg4rs工具执行失败: {}", stderr);
                // 返回原始工作目录
                if std::env::set_current_dir(&original_dir).is_err() {
                    warn!("无法返回原始工作目录: {}", original_dir.display());
                }
                return None;
            }

            // 检查JSON输出
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            info!("call-cg4rs输出长度: {}", stdout.len());

            // 记录运行call-cg4rs后的目录内容
            info!("运行call-cg4rs后的目录内容:");
            if let Ok(mut entries) = tokio_fs::read_dir(".").await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    if let Ok(path) = entry.path().into_os_string().into_string() {
                        info!("  {}", path);
                    }
                }
            }

            // 检查当前目录中的target目录（在解压后的项目目录中）
            let target_dir = Path::new("target");
            if target_dir.exists() {
                info!("target目录存在");

                // 列出target目录内容
                if let Ok(mut entries) = tokio_fs::read_dir(target_dir).await {
                    info!("target目录内容:");
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        if let Ok(path) = entry.path().into_os_string().into_string() {
                            info!("  {}", path);
                        }
                    }
                }

                // 检查callers.json文件
                let callers_json_path = target_dir.join("callers.json");
                if callers_json_path.exists() {
                    info!("callers.json文件存在");

                    // 读取callers.json文件内容
                    if let Ok(content) = tokio_fs::read_to_string(&callers_json_path).await {
                        info!("callers.json 内容:");
                        if let Ok(json) = serde_json::from_str::<Value>(&content) {
                            info!(
                                "{}",
                                serde_json::to_string_pretty(&json).unwrap_or_default()
                            );
                        } else {
                            info!("{}", content);
                        }

                        // 解析JSON并检查total_callers
                        if let Ok(json) = serde_json::from_str::<Value>(&content) {
                            if let Some(total_callers) =
                                json.get("total_callers").and_then(|v| v.as_i64())
                            {
                                if total_callers > 0 {
                                    info!(
                                        "crate {} {} 调用了函数 {} (调用者数量: {})",
                                        crate_name, crate_version, function_path, total_callers
                                    );

                                    // 保存callers.json内容
                                    let callers_content = content.clone();

                                    // 返回上级目录
                                    if std::env::set_current_dir("..").is_err() {
                                        warn!("无法返回上级目录");
                                    }

                                    // 清理下载的压缩包和解压后的项目文件夹
                                    let _ = tokio_fs::remove_file(format!(
                                        "{}-{}.crate",
                                        crate_name, crate_version
                                    ))
                                    .await;
                                    let _ = Command::new("rm")
                                        .args(&["-rf", &crate_dir.to_string_lossy()])
                                        .output()
                                        .await;

                                    // 返回原始工作目录
                                    if std::env::set_current_dir(&original_dir).is_err() {
                                        warn!("无法返回原始工作目录: {}", original_dir.display());
                                    }

                                    return Some(callers_content);
                                }

                            }

                        }
                    } else {
                        warn!("无法读取callers.json文件: {}", callers_json_path.display());
                    }
                } else {
                    warn!("callers.json文件不存在: {}", callers_json_path.display());
                }
            } else {
                warn!("target目录不存在");
            }

            // 返回上级目录
            if std::env::set_current_dir("..").is_err() {
                warn!("无法返回上级目录");
            }

            // 清理下载的压缩包和解压后的项目文件夹
            let _ = tokio_fs::remove_file(format!("{}-{}.crate", crate_name, crate_version)).await;
            let _ = Command::new("rm")
                .args(&["-rf", &crate_dir.to_string_lossy()])
                .output()
                .await;
        }

        // 返回原始工作目录
        if std::env::set_current_dir(&original_dir).is_err() {
            warn!("无法返回原始工作目录: {}", original_dir.display());
        }

        info!(
            "crate {} {} 没有调用函数 {}",
            crate_name, crate_version, function_path
        );
        None
    }

    // 处理叶子节点的函数调用分析
    async fn process_leaf_node(&self, node: &DependencyNode, target_function_path: &str) -> bool {
        info!("处理叶子节点: {} {}", node.name, node.version);

        // 检查函数调用
        if let Some(callers_json) = self
            .check_function_call(&node.name, &node.version, target_function_path)
            .await
        {
            // 获取当前工作目录的绝对路径
            let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

            // 创建结果文件名
            let result_filename = format!("{}-{}-callers.json", node.name, node.version);
            let result_path = current_dir.join("target").join(&result_filename);

            // 确保 target 目录存在
            if let Some(parent) = result_path.parent() {
                if let Err(e) = tokio_fs::create_dir_all(parent).await {
                    warn!("创建目录失败: {}", e);
                    return false;
                }
            }

            // 写入结果文件
            if let Err(e) = tokio_fs::write(&result_path, &callers_json).await {
                warn!("写入结果文件失败: {}", e);
                return false;
            }

            info!("已保存结果到: {}", result_path.display());

            return true;
        }

        false
    }

    // 处理叶子节点的函数调用分析 - 使用异步并行
    async fn process_leaf_nodes_parallel(
        &self,
        nodes: Vec<DependencyNode>,
        target_function_path: &str,
    ) {
        info!("开始并行处理 {} 个叶子节点", nodes.len());

        let analyzer = Arc::new(self.clone());
        let target_function_path = target_function_path.to_string();

        // 创建任务并发限制
        let max_concurrent_tasks = 64;

        // 使用StreamExt的buffer_unordered进行并发处理
        let results = stream::iter(nodes)
            .map(|node| {
                let analyzer = Arc::clone(&analyzer);
                let target_path = target_function_path.clone();
                async move { analyzer.process_leaf_node(&node, &target_path).await }
            })
            .buffer_unordered(max_concurrent_tasks)
            .collect::<Vec<_>>()
            .await;

        let successful = results.iter().filter(|&&result| result).count();
        info!(
            "所有叶子节点处理完成，成功处理: {}/{}",
            successful,
            results.len()
        );
    }

    // 查找所有依赖者
    pub async fn find_all_dependents(
        &self,
        start_crate: &str,
        version_range: &str,
        target_function_path: &str,
    ) -> Result<Vec<DependencyNode>> {
        info!(
            "开始BFS查找所有依赖者: {} {} 目标函数: {}",
            start_crate, version_range, target_function_path
        );

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut leaf_nodes = Vec::new();

        // 解析版本范围
        let version_req = VersionReq::parse(version_range)
            .map_err(|e| anyhow::anyhow!("解析版本范围失败: {}", e))?;

        // 获取起始crate的所有版本
        let versions = self.database.query_crate_versions(start_crate).await?;

        for version in versions {
            if let Ok(ver) = Version::parse(&version) {
                if version_req.matches(&ver) {
                    queue.push_back((start_crate.to_string(), version.clone()));
                }
            }
        }

        // BFS遍历
        while let Some((current_crate, current_version)) = queue.pop_front() {
            info!("处理 crate: {} version: {}", current_crate, current_version);

            // 查询当前crate-version的所有依赖者
            let dependents = self.database.query_dependents(&current_crate).await?;
            let mut node = DependencyNode {
                name: current_crate.clone(),
                version: current_version.clone(),
                dependents: Vec::new(),
            };

            let mut has_valid_dependents = false;

            // 处理每个依赖者
            for (dep_name, dep_version, req) in dependents {
                // 检查版本匹配
                if let (Ok(ver), Ok(dep_req)) =
                    (Version::parse(&current_version), VersionReq::parse(&req))
                {
                    if dep_req.matches(&ver) {
                        // 先检查是否调用了目标函数
                        if self
                            .check_function_call(&dep_name, &dep_version, target_function_path)
                            .await
                            .is_some()
                        {
                            info!(
                                "依赖者 {} {} 调用了目标函数 {}",
                                dep_name, dep_version, target_function_path
                            );

                            // 将匹配的依赖关系添加到当前节点
                            node.dependents
                                .push((dep_name.clone(), dep_version.clone(), req));
                            has_valid_dependents = true;

                            // 如果这个依赖者还没访问过，加入队列
                            let dep_crate_ver = CrateVersion {
                                name: dep_name.clone(),
                                version: dep_version.clone(),
                            };
                            if !visited.contains(&dep_crate_ver) {
                                info!("添加新的依赖者到队列: {} {}", dep_name, dep_version);
                                queue.push_back((dep_name, dep_version));
                                visited.insert(dep_crate_ver);
                            }
                        } else {
                            info!("依赖者 {} {} 没有调用目标函数，跳过", dep_name, dep_version);
                        }
                    }
                }
            }

            // 如果是叶子节点（没有有效的dependents），保存结果
            if !has_valid_dependents {
                info!("找到叶子节点: {} {}", node.name, node.version);
                leaf_nodes.push(node);
            }
        }

        // 并行处理所有叶子节点
        info!("开始处理 {} 个叶子节点", leaf_nodes.len());
        self.process_leaf_nodes_parallel(leaf_nodes.clone(), target_function_path)
            .await;

        info!("BFS遍历完成，找到 {} 个叶子节点", leaf_nodes.len());
        Ok(leaf_nodes)
    }
}
