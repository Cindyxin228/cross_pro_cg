#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct FunctionNode {
    crate_name: String,
    crate_version: String,
    function_path: String,
    // constraint_depth: usize,
}

impl FunctionNode {
    // 构造函数
    fn new(crate_name: String, crate_version: String, function_path: String) -> Self {
        FunctionNode {
            crate_name,
            crate_version,
            function_path,
        }
    }
}

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::{Bfs, EdgeRef};
use std::collections::{HashMap, HashSet};

#[derive(Debug)]
pub struct Graph {
    graph: DiGraph<FunctionNode, i32>, // 有向图，节点是字符串类型，边的权重为空
    node_map: HashMap<FunctionNode, NodeIndex>, 
}

impl Graph{
    pub fn new() -> Self {
        Graph {
            graph: DiGraph::new(), // 创建一个新的有向图
            node_map: HashMap::new(),
        }
    }

    // 添加节点到图中
    fn add_node(&mut self, node: FunctionNode) -> NodeIndex {
        self.graph.add_node(node)
    }

    // 添加边到图中
    fn add_edge(&mut self, from: NodeIndex, to: NodeIndex, weight:i32) {
        self.graph.add_edge(from, to, weight);
    }

    // 打印所有节点和它们的出度
    pub fn print_graph(&self) {
        for node in self.graph.node_indices() {
            let neighbors: Vec<NodeIndex> = self.graph.neighbors(node).collect();
            println!("Node {:?}: {:?}", self.graph[node], neighbors);
        }
    }

    pub fn bfs_with_edge_weights(&self, start: NodeIndex) {
        let mut bfs = Bfs::new(&self.graph, start);
        
        while let Some(node) = bfs.next(&self.graph) {
            println!("Visiting node: {:?}", self.graph[node]);

            // 获取所有出边及其权重
            for edge in self.graph.edges(node) {
                let (from, to) = (edge.source(), edge.target()); // 获取起始节点和目标节点
                let weight = edge.weight(); // 获取边的权重
                println!("Edge from {:?} to {:?} with weight: {}", from.index(), to.index(), weight);
            }
        }
    }

    fn process_dependencies(&mut self, crate_name: &str, crate_version: &str, parent_index: NodeIndex) {
        let _ = std::process::Command::new("curl")
            .args(&["-L", &format!("https://crates.io/api/v1/crates/{}/{}/download", crate_name, crate_version)])
            .output();
            
        let _ = std::process::Command::new("tar")
            .args(&["-xf", &format!("{}-{}.crate", crate_name, crate_version)])
            .output();
            
        std::env::set_current_dir(format!("{}-{}", crate_name, crate_version)).unwrap();
        
        let output = std::process::Command::new("call-cg")
            .args(&["--find-callers-of", &self.graph[parent_index].function_path])
            .output()
            .expect("Failed to execute call-cg");
            
        if let Ok(contents) = std::fs::read_to_string("./target/callers.txt") {
            for line in contents.lines() {
                let parts: Vec<&str> = line.split("--").collect();
                if parts.len() == 2 {
                    let (caller, callee) = (parts[0], parts[1]);
                }
            }
        }
        
        std::env::set_current_dir("..").unwrap();
    }

    pub fn build_from_cve(&mut self, cve: FunctionNode) {
        let download_url = format!("https://crates.io/api/v1/crates/{}/{}/download", 
            cve.crate_name, cve.crate_version);
        let crate_file = format!("{}-{}.crate", cve.crate_name, cve.crate_version);
        
        let vulnerable_index = self.add_node(cve.clone()); // 使用 clone() 以避免移动
        let mut visited: HashSet<FunctionNode> = HashSet::new();
        self.build_graph_recursive(cve, &mut visited, vulnerable_index);
    }

    // pub fn build(&mut self) {
    //     // 添加节点和边
    //     let weight = 1;
    //     let parent = self.graph.add_node(FunctionNode::new("a".to_string(), "b".to_string(), "c".to_string()));
    //     let child = self.graph.add_node(FunctionNode::new("A".to_string(), "B".to_string(), "C".to_string()));
    //     self.graph.add_edge(parent, child, weight);
        
    
    //     // BFS 遍历
    //     // let mut bfs = Bfs::new(&self.graph, parent);
    //     // while let Some(node) = bfs.next(&self.graph) {
    //     //     println!("Visiting: {:?}", node);
    //     // }
    // }

    // 模拟解析函数信息
    fn parse_function(&self, function: &str) -> FunctionNode {
        // 这里假设我们能从函数名解析到对应的 FunctionNode
        // 你可以用自己的方式来解析函数及其版本等信息
        FunctionNode {
            crate_name: "example_crate".to_string(),
            crate_version: "1.0.0".to_string(),
            function_path: function_name.to_string(),
        }
    }

    // 模拟获取依赖的函数
    fn get_dependencies(&self, function_name: &str) -> Vec<FunctionNode> {
        // 这里你需要根据函数名返回它的依赖
        // 假设每个函数有一个类似这种的依赖列表
        match function_name {
            "ring::digest::digest" => vec![
                "ring::digest::Context::finish".to_string(),
                "ring::digest::Context::new".to_string(),
                "ring::digest::Context::update".to_string(),
            ],
            "<ring::digest::Digest as core::convert::AsRef<[u8]>>::as_ref" => vec![
                "<[ring::endian::BigEndian<u64>; 8] as ring::endian::ArrayEncoding<[u8; ring::::endian::{impl#36}::{constant#0}]>>::as_byte_array".to_string(),
            ],
            // 其他函数依赖
            _ => vec![],
        }
    }

    fn build_graph_recursive(&mut self, function: FunctionNode, visited: &mut HashSet<FunctionNode>, nodeIndex: NodeIndex) {
        if visited.contains(&function) {
            return; // 如果已经访问过该函数，跳过
        }
        visited.insert(function);

        // 这里需要解析函数名和依赖关系
        // 假设你有一个解析函数来返回当前函数的依赖列表
        let function_node = self.parse_function(function);
        let current_node = self.add_node(function_node);

        // 查找当前函数的所有依赖
        let dependencies = self.get_dependencies(function); // 这个方法需要你自己实现

        for dependency in dependencies {
            if !visited.contains(&dependency) {
                let dependency_node = self.parse_function(&dependency); // 创建依赖的节点
                let dependency_index = self.add_node(dependency_node);

                // 添加边，假设边的权重是 0
                self.add_edge(current_node, dependency_index, 0);
                
                // 递归构建依赖的子图
                self.build_graph_recursive(dependency, visited);
            }
        }
    }

}


