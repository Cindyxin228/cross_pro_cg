use std::process::Command;
use std::collections::{HashSet, VecDeque};
use semver::{Version, VersionReq};
use tracing::{info, warn, error, debug, trace};

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
        
        // 修改SQL查询，使用子查询来确保我们找到正确的依赖关系
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
        info!("原始数据库查询结果: \n{}", output_str);
        
        // 解析输出
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
        if dependents.is_empty() {
            warn!("没有找到任何依赖者，SQL查询可能需要调整");
        }
        dependents
    }

    // 检查版本是否匹配版本要求
    fn version_matches_requirement(&self, version: &str, req: &str) -> bool {
        debug!("检查版本 {} 是否匹配要求 {}", version, req);
        
        match VersionReq::parse(req) {
            Ok(version_req) => {
                match Version::parse(version) {
                    Ok(ver) => {
                        let matches = version_req.matches(&ver);
                        debug!("版本 {} 是否匹配要求 {}: {}", version, req, matches);
                        matches
                    },
                    Err(e) => {
                        warn!("解析版本 {} 失败: {}", version, e);
                        false
                    }
                }
            },
            Err(e) => {
                warn!("解析版本要求 {} 失败: {}", req, e);
                false
            }
        }
    }

    // 查找所有依赖者
    pub fn find_all_dependents(&mut self, start_crate: &str, version_range: &str, target_function_path: &str) -> Vec<DependencyNode> {
        info!("开始BFS查找所有依赖者: {} {} 目标函数: {}", start_crate, version_range, target_function_path);
        
        // 用于存储已访问的crate-version对，避免重复访问
        let mut visited = HashSet::new();
        // BFS队列
        let mut queue = VecDeque::new();
        // 存储叶子节点（没有dependents的节点）
        let mut leaf_nodes = Vec::new();
        // 存储起始crate的精确版本
        let mut start_crate_versions = Vec::new();
        
        // 1. 解析初始版本范围
        let version_req = match VersionReq::parse(version_range) {
            Ok(req) => req,
            Err(e) => {
                error!("解析版本范围失败: {}", e);
                return Vec::new();
            }
        };
        
        // 2. 获取起始crate的所有版本并加入队列
        let versions = self.query_crate_versions(start_crate);
        for version in versions {
            if let Ok(ver) = Version::parse(&version) {
                if version_req.matches(&ver) {
                    let crate_ver = CrateVersion {
                        name: start_crate.to_string(),
                        version: version.clone(),
                    };
                    if !visited.contains(&crate_ver) {
                        queue.push_back((start_crate.to_string(), version.clone()));
                        visited.insert(crate_ver);
                        start_crate_versions.push(version);
                    }
                }
            }
        }
        
        // 3. BFS遍历
        while let Some((current_crate, current_version)) = queue.pop_front() {
            info!("处理 crate: {} version: {}", current_crate, current_version);
            
            // 检查当前crate是否调用了目标函数
            if !self.check_function_call(&current_crate, &current_version, target_function_path) {
                info!("crate {} {} 没有调用目标函数 {}，跳过", current_crate, current_version, target_function_path);
                continue;
            }
            
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
            
            // 如果是叶子节点（没有有效的dependents），加入结果集
            if !has_valid_dependents {
                info!("找到叶子节点: {} {}", node.name, node.version);
                leaf_nodes.push(node);
            }
        }
        
        info!("BFS遍历完成，找到 {} 个叶子节点", leaf_nodes.len());
        
        // 4. 分析叶子节点并保存结果
        for leaf_node in &leaf_nodes {
            // 对于每个叶子节点，我们需要分析它与每个起始crate版本的关系
            for start_version in &start_crate_versions {
                self.analyze_leaf_node(leaf_node, start_crate, start_version, target_function_path);
            }
        }
        
        leaf_nodes
    }
    
    // 检查crate是否调用了指定的函数
    fn check_function_call(&self, crate_name: &str, crate_version: &str, function_path: &str) -> bool {
        info!("检查 crate {} {} 是否调用了函数 {}", crate_name, crate_version, function_path);
        
        // 1. 下载并解压crate
        let crate_file = format!("{}-{}.crate", crate_name, crate_version);
        let _ = Command::new("curl")
            .args(&["-L", &format!("https://crates.io/api/v1/crates/{}/{}/download", crate_name, crate_version)])
            .output();
        let _ = Command::new("tar")
            .args(&["-xf", &crate_file])
            .output();
            
        // 2. 进入crate目录
        if std::env::set_current_dir(format!("{}-{}", crate_name, crate_version)).is_err() {
            warn!("无法进入crate目录: {}-{}", crate_name, crate_version);
            return false;
        }
        
        // 3. 运行call-cg工具
        let _ = Command::new("call-cg")
            .args(&["--find-callers", function_path])
            .output();
            
        // 4. 检查callers.txt是否存在且有内容
        let result = std::fs::read_to_string("./target/callers.txt")
            .map(|contents| !contents.trim().is_empty())
            .unwrap_or(false);
        
        // 5. 返回上级目录
        let _ = std::env::set_current_dir("..");
        
        // 6. 清理下载的压缩包和解压后的项目文件夹
        let _ = std::fs::remove_file(&crate_file);
        let _ = Command::new("rm")
            .args(&["-rf", &format!("{}-{}", crate_name, crate_version)])
            .output();
        
        info!("crate {} {} 调用函数 {} 的结果: {}", crate_name, crate_version, function_path, result);
        result
    }

    // 分析叶子节点并保存结果
    fn analyze_leaf_node(&self, leaf_node: &DependencyNode, start_crate: &str, start_version: &str, target_function_path: &str) {
        info!("分析叶子节点: {} {} 与起始crate: {} {}", leaf_node.name, leaf_node.version, start_crate, start_version);
        
        // 1. 下载并解压crate
        let crate_file = format!("{}-{}.crate", leaf_node.name, leaf_node.version);
        let _ = Command::new("curl")
            .args(&["-L", &format!("https://crates.io/api/v1/crates/{}/{}/download", leaf_node.name, leaf_node.version)])
            .output();
        let _ = Command::new("tar")
            .args(&["-xf", &crate_file])
            .output();
            
        // 2. 进入crate目录
        if std::env::set_current_dir(format!("{}-{}", leaf_node.name, leaf_node.version)).is_err() {
            warn!("无法进入crate目录: {}-{}", leaf_node.name, leaf_node.version);
            return;
        }
        
        // 3. 运行call-cg工具
        let _ = Command::new("call-cg")
            .args(&["--find-callers", target_function_path])
            .output();
            
        // 4. 检查callers.txt是否存在
        if let Ok(_) = std::fs::read_to_string("./target/callers.txt") {
            // 5. 复制并重命名文件
            let output_filename = format!("caller({}_{})_callee({}_{}).txt", leaf_node.name, leaf_node.version, start_crate, start_version);
            let _ = Command::new("cp")
                .args(&["./target/callers.txt", &output_filename])
                .output();
            
            info!("已保存分析结果到文件: {}", output_filename);
        } else {
            warn!("叶子节点 {} {} 没有调用目标函数 {}", leaf_node.name, leaf_node.version, target_function_path);
        }
        
        // 6. 返回上级目录
        let _ = std::env::set_current_dir("..");
        
        // 7. 清理下载的压缩包和解压后的项目文件夹
        let _ = std::fs::remove_file(&crate_file);
        let _ = Command::new("rm")
            .args(&["-rf", &format!("{}-{}", leaf_node.name, leaf_node.version)])
            .output();
    }
}