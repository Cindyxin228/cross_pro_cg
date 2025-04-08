use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct FunctionNode {
    pub crate_name: String,
    pub crate_version: String,
    pub function_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CallerInfo {
    name: String,
    version: String,
    path: String,
    constraint_depth: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct CallersJson {
    caller: CallerInfo,
    callee: Vec<CallerInfo>,
}

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::Bfs;

#[derive(Debug)]
pub struct Graph {
    graph: DiGraph<FunctionNode, usize>, // 边权重为约束深度
    // 缓存：(crate_name, crate_version) -> Vec<(function_path, NodeIndex)>
    function_cache: HashMap<(String, String), Vec<(String, NodeIndex)>>,
}

impl Graph {
    pub fn new() -> Self {
        Graph {
            graph: DiGraph::new(),
            function_cache: HashMap::new(),
        }
    }

    fn add_node(&mut self, node: FunctionNode) -> NodeIndex {
        let node_index = self.graph.add_node(node.clone());
        
        // 更新缓存
        let cache_key = (node.crate_name.clone(), node.crate_version.clone());
        self.function_cache
            .entry(cache_key)
            .or_insert_with(Vec::new)
            .push((node.function_path.clone(), node_index));
            
        node_index
    }

    fn add_edge(&mut self, from: NodeIndex, to: NodeIndex, constraint_depth: usize) {
        self.graph.add_edge(from, to, constraint_depth);
    }

    pub fn print_graph(&self) {
        for node in self.graph.node_indices() {
            let neighbors: Vec<NodeIndex> = self.graph.neighbors(node).collect();
            println!("Node {:?}: {:?}", self.graph[node], neighbors);
        }
    }

    // 共通的下载和分析函数
    fn download_and_analyze(&self, crate_name: &str, crate_version: &str, function_path: &str) -> Option<CallersJson> {
        // 1. 下载并解压crate
        let crate_file = format!("{}-{}.crate", crate_name, crate_version);
        let _ = std::process::Command::new("curl")
            .args(&["-L", &format!("https://crates.io/api/v1/crates/{}/{}/download", crate_name, crate_version)])
            .output();
        let _ = std::process::Command::new("tar")
            .args(&["-xf", &crate_file])
            .output();
            
        // 2. 进入crate目录
        if std::env::set_current_dir(format!("{}-{}", crate_name, crate_version)).is_err() {
            return None;
        }
        
        // 3. 运行call-cg工具
        let _ = std::process::Command::new("call-cg")
            .args(&["--find-callers", function_path])
            .output();
            
        // 4. 解析callers.txt
        let result = std::fs::read_to_string("./target/callers.txt")
            .ok()
            .and_then(|contents| serde_json::from_str::<CallersJson>(&contents).ok());
        
        // 5. 返回上级目录
        let _ = std::env::set_current_dir("..");
        
        result
    }

    pub fn process_upstream_function(&mut self, crate_name: &str, crate_version: &str, function_path: &str, node_index: NodeIndex) {
        if let Some(callers_json) = self.download_and_analyze(crate_name, crate_version, function_path) {
            for callee in callers_json.callee {
                // 只处理同一个crate内的调用
                if callee.name == crate_name {
                    let callee_node = FunctionNode {
                        crate_name: callee.name.clone(),
                        crate_version: callee.version.clone(),
                        function_path: callee.path.clone(),
                    };
                    
                    let callee_index = self.add_node(callee_node);
                    self.add_edge(node_index, callee_index, callee.constraint_depth);
                    
                    self.process_upstream_function(
                        &callee.name,
                        &callee.version,
                        &callee.path,
                        callee_index
                    );
                }
            }
        }
    }

    pub fn process_downstream_function(&mut self, crate_name: &str, crate_version: &str, upstream_function: &str, upstream_node_index: NodeIndex) {
        if let Some(callers_json) = self.download_and_analyze(crate_name, crate_version, upstream_function) {
            for callee in callers_json.callee {
                let callee_node = FunctionNode {
                    crate_name: callee.name.clone(),
                    crate_version: callee.version.clone(),
                    function_path: callee.path.clone(),
                };
                
                let callee_index = self.add_node(callee_node);
                self.add_edge(upstream_node_index, callee_index, callee.constraint_depth);
            }
        }
    }

    // 从缓存中获取crate的所有函数节点
    fn get_crate_functions_from_cache(&self, crate_name: &str, crate_version: &str) -> Option<&Vec<(String, NodeIndex)>> {
        self.function_cache.get(&(crate_name.to_string(), crate_version.to_string()))
    }

    // 修改后的analyze_downstream函数
    pub fn analyze_downstream(&mut self, upstream_crate: &str, upstream_version: &str, downstream_crates: &Vec<(String, String)>) {
        // 先收集所有需要的信息
        let upstream_functions: Vec<(String, NodeIndex)> = if let Some(functions) = self.get_crate_functions_from_cache(upstream_crate, upstream_version) {
            functions.clone()
        } else {
            return;
        };

        // 对每个下游crate，检查它是否调用了上游crate中图中的任何函数
        for (downstream_name, downstream_version) in downstream_crates {
            // 对上游crate中图中的每个函数，检查下游crate是否有调用
            for (upstream_path, upstream_index) in &upstream_functions {
                if let Some(callers_json) = self.download_and_analyze(
                    downstream_name,
                    downstream_version,
                    upstream_path
                ) {
                    for callee in callers_json.callee {
                        if callee.name == *downstream_name {
                            let callee_node = FunctionNode {
                                crate_name: callee.name.clone(),
                                crate_version: callee.version.clone(),
                                function_path: callee.path.clone(),
                            };
                            
                            let callee_index = self.add_node(callee_node);
                            self.add_edge(*upstream_index, callee_index, callee.constraint_depth);
                        }
                    }
                }
            }
        }
    }


    pub fn get_crate_functions(&self, crate_name: &str, crate_version: &str) -> Vec<FunctionNode> {
        if let Some(cached_functions) = self.get_crate_functions_from_cache(crate_name, crate_version) {
            cached_functions
                .iter()
                .map(|(_, node_index)| self.graph[*node_index].clone())
                .collect()
        } else {
            Vec::new()
        }
    }

    // 统一的函数调用图分析函数
    pub fn analyze_function_calls(&mut self, json_file: &str) -> std::io::Result<()> {
        // 读取JSON文件
        let file = File::open(json_file)?;
        let reader = BufReader::new(file);
        let dependencies: Vec<DependencyInfo> = serde_json::from_reader(reader)?;

        // 处理第一个crate（CVE所在的crate）
        if let Some(cve_crate) = dependencies.first() {
            // 创建CVE函数节点
            let cve_node = FunctionNode {
                crate_name: cve_crate.crate_name.clone(),
                crate_version: cve_crate.version.clone(),
                function_path: cve_crate.function.clone(), // 需要在DependencyInfo中添加这个字段
            };
            
            let cve_index = self.add_node(cve_node);
            
            // 分析CVE所在crate的内部调用
            self.process_upstream_function(
                &cve_crate.crate_name,
                &cve_crate.version,
                &cve_crate.cve_function,
                cve_index
            );

            // 分析依赖关系
            for dep_info in dependencies {
                let crate_name = &dep_info.crate_name;
                let crate_version = &dep_info.version;
                
                // 获取当前crate在图中的所有函数
                let functions = self.get_crate_functions(crate_name, crate_version);
                
                // 分析每个函数
                for func in functions {
                    // 分析直接依赖的下游crates
                    self.analyze_downstream(
                        crate_name,
                        crate_version,
                        &dep_info.dependents
                    );
                }
            }
        }

        Ok(())
    }
}

// 修改DependencyInfo结构体
#[derive(Debug, Serialize, Deserialize)]
struct DependencyInfo {
    crate_name: String,
    version: String,
    dependents: Vec<(String, String)>, // (crate_name, version)
    cve_function: Option<String>, // 如果是CVE所在的crate，这个字段会有值
} 