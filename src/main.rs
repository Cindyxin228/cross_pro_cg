mod callgraph;
use callgraph::{Graph, FunctionNode};
fn main() {
    let mut graph = Graph::new();

    // 1. 分析上游CVE所在crate的内部调用
    let vulnerable_node = FunctionNode {
        crate_name: "vulnerable_crate".to_string(),
        crate_version: "1.0.0".to_string(),
        function_path: "vulnerable_function".to_string(),
    };
    graph.process_upstream_function(&vulnerable_node.crate_name, &vulnerable_node.crate_version, &vulnerable_node.function_path, 0.into());

    // // 2. 手动指定直接依赖的下游crates
    // let direct_dependents = vec![
    //     ("dependent_crate1".to_string(), "0.1.0".to_string()),
    //     ("dependent_crate2".to_string(), "0.2.0".to_string()),
    // ];

    // // 3. 分析直接依赖的下游crates中对vulnerable_function的调用
    // let downstream_functions = graph.analyze_downstream(&vulnerable_node, &direct_dependents);

    // // 4. 如果需要继续分析下一层依赖，可以手动指定新的下游crates
    // for downstream_func in downstream_functions {
    //     let new_dependents = vec![
    //         // 手动指定依赖于downstream_func.crate_name的crates
    //     ];
    //     graph.analyze_downstream(&downstream_func, &new_dependents);
    // }
}