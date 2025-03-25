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

#[derive(Debug)]
pub struct Graph {
    graph: DiGraph<FunctionNode, i32>, // 有向图，节点是字符串类型，边的权重为空
    root: Option<NodeIndex>, // 存储根节点
}

impl Graph{
    pub fn new() -> Self {
        Graph {
            graph: DiGraph::new(), // 创建一个新的有向图
            root: None,
        }
    }

    // 设置根节点
    fn set_root(&mut self, root: NodeIndex) {
        self.root = Some(root);
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

    pub fn build(&mut self) {
        // 添加节点和边
        let weight = 1;
        let parent = self.graph.add_node(FunctionNode::new("a".to_string(), "b".to_string(), "c".to_string()));
        let child = self.graph.add_node(FunctionNode::new("A".to_string(), "B".to_string(), "C".to_string()));
        self.graph.add_edge(parent, child, weight);
        
    
        // BFS 遍历
        let mut bfs = Bfs::new(&self.graph, parent);
        while let Some(node) = bfs.next(&self.graph) {
            println!("Visiting: {:?}", node);
        }
    }
}


