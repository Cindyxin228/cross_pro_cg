use std::process::Command;
use std::collections::{HashSet, VecDeque};
use semver::{Version, VersionReq};
use tracing::{info, warn, error, debug, trace};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use serde_json::Value;

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

#[derive(Debug)]
pub struct DependencyAnalyzer {
    db_user: String,
    db_name: String,
}

impl DependencyAnalyzer {
    pub fn new(db_user: &str, db_name: &str) -> Self {
        info!("初始化 DependencyAnalyzer: db_user={}, db_name={}", db_user, db_name);
        
        DependencyAnalyzer {
            db_user: db_user.to_string(),
            db_name: db_name.to_string(),
        }
    }

    // 查询某个crate的所有版本
    fn query_crate_versions(&self, crate_name: &str) -> Vec<String> {
        info!("查询crate {} 的所有版本", crate_name);
        
        let query = format!(
            "SELECT DISTINCT v.num 
             FROM versions v 
             JOIN crates c ON v.crate_id = c.id 
             WHERE c.name = '{}' 
             ORDER BY v.num;",
            crate_name
        );

        info!("执行SQL查询: {}", query);

        let output = Command::new("psql")
            .args(&[
                "-U", &self.db_user,
                "-d", &self.db_name,
                "-c", &query,
                "-t"
            ])
            .output()
            .expect("Failed to execute psql command");

        let output_str = String::from_utf8_lossy(&output.stdout);
        info!("数据库查询结果: {}", output_str);
        
        let versions: Vec<String> = output_str
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
            .collect();
            
        info!("找到 {} 个版本: {:?}", versions.len(), versions);
        versions
    }

    // 查询依赖某个crate的所有crates
    fn query_dependents(&self, crate_name: &str) -> Vec<(String, String, String)> {
        info!("查询依赖 {} 的所有crates", crate_name);
        
        let query = format!(
            "WITH target_crate AS (
                SELECT id FROM crates WHERE name = '{}'
            )
            SELECT DISTINCT c.name, v.num, d.req
            FROM dependencies d
            JOIN versions v ON d.version_id = v.id
            JOIN crates c ON v.crate_id = c.id
            WHERE d.crate_id = (SELECT id FROM target_crate)
            AND d.req IS NOT NULL
            ORDER BY c.name, v.num;",
            crate_name
        );

        info!("执行SQL查询: {}", query);

        let output = Command::new("psql")
            .args(&[
                "-U", &self.db_user,
                "-d", &self.db_name,
                "-c", &query,
                "-t"
            ])
            .output()
            .expect("Failed to execute psql command");

        let output_str = String::from_utf8_lossy(&output.stdout);
        //info!("原始数据库查询结果: \n{}", output_str);
        
        let dependents: Vec<(String, String, String)> = output_str
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() == 3 {
                    let name = parts[0].trim().to_string();
                    let version = parts[1].trim().to_string();
                    let req = parts[2].trim().to_string();
                    Some((name, version, req))
                } else {
                    warn!("无法解析行: {}", line);
                    None
                }
            })
            .collect();
            
        info!("找到 {} 个依赖者", dependents.len());
        dependents
    }

    // 下载并解压crate
    fn download_and_extract_crate(&self, crate_name: &str, crate_version: &str) -> bool {
        info!("下载并解压 crate: {} {}", crate_name, crate_version);
        
        // 使用临时目录
        let temp_dir = std::env::temp_dir();
        info!("使用临时目录: {}", temp_dir.display());
        
        // 切换到临时目录
        if std::env::set_current_dir(&temp_dir).is_err() {
            warn!("无法切换到临时目录: {}", temp_dir.display());
            return false;
        }
        
        let crate_file = format!("{}-{}.crate", crate_name, crate_version);
        let crate_dir = format!("{}-{}", crate_name, crate_version);
        
        // 如果目录已存在，先删除
        if Path::new(&crate_dir).exists() {
            info!("目录 {} 已存在，先删除", crate_dir);
            if let Err(e) = fs::remove_dir_all(&crate_dir) {
                warn!("删除目录 {} 失败: {}", crate_dir, e);
                return false;
            }
        }
        
        // 如果文件已存在，先删除
        if Path::new(&crate_file).exists() {
            info!("文件 {} 已存在，先删除", crate_file);
            if let Err(e) = fs::remove_file(&crate_file) {
                warn!("删除文件 {} 失败: {}", crate_file, e);
                return false;
            }
        }
        
        // 下载crate
        info!("下载 crate: {}", crate_file);
        // let download_cmd = format!("curl -L https://crates.io/api/v1/crates/{}/{}/download -o {}", 
        //     crate_name, crate_version, crate_file);
        //info!("执行下载命令: {}", download_cmd);
        let curl_path = "/usr/bin/curl"; 
        let download_result = Command::new(curl_path)
            .args(&["-L", &format!("https://crates.io/api/v1/crates/{}/{}/download", crate_name, crate_version)])
            .arg("-o")
            .arg(&crate_file)
            .output();
            
        if let Err(e) = download_result {
            warn!("下载 crate {} 失败: {}", crate_file, e);
            return false;
        }

        // if let Ok(output) = download_result {
        //     if !output.status.success() {
        //         let stderr = String::from_utf8_lossy(&output.stderr);
        //         let stdout = String::from_utf8_lossy(&output.stdout);
        //         warn!("curl 命令失败，错误信息: {}", stderr);
        //         warn!("curl 命令的输出: {}", stdout);
        //         return false;
        //     }
        // } else {
        //     warn!("下载命令执行失败: {}", download_result.err().unwrap());
        //     return false;
        // }
        
        // 检查下载的文件是否存在
        if !Path::new(&crate_file).exists() {
            warn!("下载后文件 {} 不存在", crate_file);
            return false;
        }
        
        // 检查文件大小
        if let Ok(metadata) = fs::metadata(&crate_file) {
            let size = metadata.len();
            info!("下载的文件大小: {} 字节", size);
            if size == 0 {
                warn!("下载的文件大小为0，可能下载失败");
                return false;
            }
        } else {
            warn!("无法获取文件 {} 的元数据", crate_file);
            return false;
        }
        
        // 解压crate
        info!("解压 crate: {}", crate_file);
        //let extract_cmd = format!("tar -xf {}", crate_file);
        //info!("执行解压命令: {}", extract_cmd);
        
        let extract_result = Command::new("tar")
            .args(&["-xf", &crate_file])
            .output();
            
        if let Err(e) = extract_result {
            warn!("解压 crate {} 失败: {}", crate_file, e);
            return false;
        }
        
        // 检查解压命令的输出
        if let Ok(output) = extract_result {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("解压命令失败: {}", stderr);
                return false;
            }
        }
        
        // 检查目录是否存在
        if !Path::new(&crate_dir).exists() {
            warn!("解压后目录 {} 不存在", crate_dir);
            
            // 尝试列出当前目录内容
            if let Ok(entries) = fs::read_dir(".") {
                info!("当前目录内容:");
                for entry in entries {
                    if let Ok(entry) = entry {
                        if let Ok(path) = entry.path().into_os_string().into_string() {
                            info!("  {}", path);
                        }
                    }
                }
            }
            
            return false;
        }
        
        info!("成功下载并解压 crate: {} {}", crate_name, crate_version);
        true
    }

    // 检查crate是否调用了指定的函数，并返回callers.txt的内容
    fn check_function_call(&self, crate_name: &str, crate_version: &str, function_path: &str) -> Option<String> {
        info!("检查 crate {} {} 是否调用了函数 {}", crate_name, crate_version, function_path);
        
        // 下载并解压crate
        if !self.download_and_extract_crate(crate_name, crate_version) {
            warn!("无法下载或解压 crate: {} {}", crate_name, crate_version);
            return None;
        }
        
        let crate_dir = format!("{}-{}", crate_name, crate_version);
        
        // 进入crate目录
        if std::env::set_current_dir(&crate_dir).is_err() {
            warn!("无法进入crate目录: {}", crate_dir);
            return None;
        }
        
        // 运行call-cg4rs工具
        info!("运行call-cg4rs工具");
        let call_cg_result = Command::new("call-cg4rs")
            .args(&["--find-callers-of", function_path, "--json-output"])
            .output();
            
        if let Err(e) = call_cg_result {
            warn!("运行call-cg4rs工具失败: {}", e);
            let _ = std::env::set_current_dir("..");
            return None;
        }
        
        if let Ok(output) = call_cg_result {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("call-cg4rs工具执行失败: {}", stderr);
                let _ = std::env::set_current_dir("..");
                return None;
            }
            
            // 检查JSON输出
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            info!("call-cg4rs输出: {}", stdout);
            
            // 解析JSON并检查total_callers
            if let Ok(json) = serde_json::from_str::<Value>(&stdout) {
                if let Some(total_callers) = json.get("total_callers").and_then(|v| v.as_i64()) {
                    if total_callers > 0 {
                        info!("crate {} {} 调用了函数 {} (调用者数量: {})", 
                            crate_name, crate_version, function_path, total_callers);
                        
                        // 返回上级目录
                        if std::env::set_current_dir("..").is_err() {
                            warn!("无法返回上级目录");
                        }
                        
                        // 清理下载的压缩包和解压后的项目文件夹
                        let _ = std::fs::remove_file(format!("{}-{}.crate", crate_name, crate_version));
                        let _ = Command::new("rm")
                            .args(&["-rf", &crate_dir])
                            .output();
                        
                        return Some(stdout);
                    }
                }
            }
            
            // 返回上级目录
            if std::env::set_current_dir("..").is_err() {
                warn!("无法返回上级目录");
            }
            
            // 清理下载的压缩包和解压后的项目文件夹
            let _ = std::fs::remove_file(format!("{}-{}.crate", crate_name, crate_version));
            let _ = Command::new("rm")
                .args(&["-rf", &crate_dir])
                .output();
        }
        
        info!("crate {} {} 没有调用函数 {}", crate_name, crate_version, function_path);
        None
    }

    // 处理叶子节点的函数调用分析
    fn process_leaf_node(&self, node: &DependencyNode, target_function_path: &str) -> bool {
        info!("处理叶子节点: {} {}", node.name, node.version);
        
        // 检查函数调用
        if let Some(callers_json) = self.check_function_call(&node.name, &node.version, target_function_path) {
            // 创建结果文件名
            let result_filename = format!("{}-{}-callers.txt", node.name, node.version);
            let result_path = Path::new("target").join(&result_filename);
            
            // 确保target目录存在
            if let Err(e) = std::fs::create_dir_all("target") {
                warn!("创建target目录失败: {}", e);
                return false;
            }
            
            // 写入结果文件
            if let Err(e) = std::fs::write(&result_path, callers_json) {
                warn!("写入结果文件失败: {}", e);
                return false;
            }
            
            info!("已保存结果到: {}", result_path.display());
            return true;
        }
        
        false
    }

    // 查找所有依赖者
    pub fn find_all_dependents(&mut self, start_crate: &str, version_range: &str, target_function_path: &str) -> Vec<DependencyNode> {
        info!("开始BFS查找所有依赖者: {} {} 目标函数: {}", start_crate, version_range, target_function_path);
        
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut leaf_nodes = Vec::new();
        
        // 解析版本范围
        let version_req = match VersionReq::parse(version_range) {
            Ok(req) => req,
            Err(e) => {
                error!("解析版本范围失败: {}", e);
                return Vec::new();
            }
        };
        
        // 获取起始crate的所有版本
        let versions = self.query_crate_versions(start_crate);
        for version in versions {
            if let Ok(ver) = Version::parse(&version) {
                if version_req.matches(&ver) {
                    // 将匹配的版本加入队列
                    queue.push_back((start_crate.to_string(), version.clone()));
                }
            }
        }
        
        // BFS遍历
        while let Some((current_crate, current_version)) = queue.pop_front() {
            info!("处理 crate: {} version: {}", current_crate, current_version);
            
            // 查询当前crate-version的所有依赖者
            let dependents = self.query_dependents(&current_crate);
            let mut node = DependencyNode {
                name: current_crate.clone(),
                version: current_version.clone(),
                dependents: Vec::new(),
            };
            
            let mut has_valid_dependents = false;
            
            // 处理每个依赖者
            for (dep_name, dep_version, req) in dependents {
                // 检查版本匹配
                if let (Ok(ver), Ok(dep_req)) = (Version::parse(&current_version), VersionReq::parse(&req)) {
                    if dep_req.matches(&ver) {
                        // 将匹配的依赖关系添加到当前节点
                        node.dependents.push((dep_name.clone(), dep_version.clone(), req));
                        has_valid_dependents = true;
                        
                        // 先检查是否调用了目标函数
                        if self.check_function_call(&dep_name, &dep_version, target_function_path).is_some() {
                            info!("依赖者 {} {} 调用了目标函数 {}", dep_name, dep_version, target_function_path);
                            let dep_node = DependencyNode {
                                name: dep_name.clone(),
                                version: dep_version.clone(),
                                dependents: Vec::new(),
                            };
                            leaf_nodes.push(dep_node);
                        }
                        
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
                    }
                }
            }
            
            // 如果是叶子节点（没有有效的dependents），检查是否调用了目标函数
            if !has_valid_dependents {
                info!("找到叶子节点: {} {}", node.name, node.version);
                
                // 检查是否调用了目标函数
                if self.check_function_call(&node.name, &node.version, target_function_path).is_some() {
                    info!("叶子节点 {} {} 调用了目标函数 {}", node.name, node.version, target_function_path);
                    leaf_nodes.push(node);
                } else {
                    info!("叶子节点 {} {} 没有调用目标函数 {}", node.name, node.version, target_function_path);
                }
            }
        }
        
        // 处理所有叶子节点
        info!("开始处理 {} 个叶子节点", leaf_nodes.len());
        for node in &leaf_nodes {
            self.process_leaf_node(node, target_function_path);
        }
        
        info!("BFS遍历完成，找到 {} 个叶子节点", leaf_nodes.len());
        leaf_nodes
    }
}