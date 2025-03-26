use serde::{Deserialize, Serialize};

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
}

impl Graph {
    pub fn new() -> Self {
        Graph {
            graph: DiGraph::new(),
        }
    }

    fn add_node(&mut self, node: FunctionNode) -> NodeIndex {
        self.graph.add_node(node)
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
            .args(&["--find-callers-of", function_path])
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

    // 修改后的analyze_downstream函数
    pub fn analyze_downstream(&mut self, upstream_crate: &str, upstream_version: &str, downstream_crates: &Vec<(String, String)>) {
        // 获取当前图中属于upstream_crate的所有函数节点
        let mut upstream_functions = Vec::new();
        for node_index in self.graph.node_indices() {
            let node = &self.graph[node_index];
            if node.crate_name == upstream_crate && node.crate_version == upstream_crate {
                upstream_functions.push((node_index, node.clone()));
            }
        }

        // 对每个下游crate，检查它是否调用了上游crate中图中的任何函数
        for (downstream_name, downstream_version) in downstream_crates {
            // 对上游crate中图中的每个函数，检查下游crate是否有调用
            for (upstream_index, upstream_func) in &upstream_functions {
                if let Some(callers_json) = self.download_and_analyze(
                    downstream_name,
                    downstream_version,
                    &upstream_func.function_path
                ) {
                    for callee in callers_json.callee {
                        // 只处理来自当前下游crate的调用
                        if callee.name == *downstream_name {
                            let callee_node = FunctionNode {
                                crate_name: callee.name.clone(),
                                crate_version: callee.version.clone(),
                                function_path: callee.path.clone(),
                            };
                            
                            let callee_index = self.add_node(callee_node);
                            // 从上游函数节点连接到下游函数节点
                            self.add_edge(*upstream_index, callee_index, callee.constraint_depth);
                        }
                    }
                }
            }
        }
    }

    // 获取某个crate在图中的所有函数节点
    pub fn get_crate_functions(&self, crate_name: &str) -> Vec<FunctionNode> {
        let mut functions = Vec::new();
        for node_index in self.graph.node_indices() {
            let node = &self.graph[node_index];
            if node.crate_name == crate_name {
                functions.push(node.clone());
            }
        }
        functions
    }
} 