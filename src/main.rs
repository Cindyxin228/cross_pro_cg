mod build_graph;


fn main(){
    let mut callgraph = build_graph::Graph::new();
    callgraph.build();
    callgraph.bfs_with_edge_weights();
}