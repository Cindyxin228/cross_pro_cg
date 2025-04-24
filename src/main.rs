mod database;
mod dependency_analyzer;
mod krate;
mod logger;

use dependency_analyzer::DependencyAnalyzer;
use std::fs;
use std::path::Path;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();

    // 手动删除日志文件
    let log_file_path = Path::new("logs/cross_pro_cg.log");
    if log_file_path.exists() {
        fs::remove_file(log_file_path).expect("无法删除旧日志文件");
    }

    let _guard = logger::log_init();

    tracing::info!("开始分析依赖关系");
    let analyzer = DependencyAnalyzer::new().await.unwrap();

    // 分析 ring crate 的依赖关系
    tracing::info!("分析 ring crate 的依赖关系");
    let leaf_nodes = analyzer
        .find_all_dependents(
            "crossbeam-channel",
            ">=0.5.11, <0.5.15",
            "crossbeam_channel::flavors::list::Channel::discard_all_messages",
        )
        .await;

    let leaf_nodes = leaf_nodes.unwrap();
    // 输出结果
    tracing::info!("找到的叶子节点数量: {}", leaf_nodes.len());
    tracing::info!("叶子节点列表:");
    for node in leaf_nodes {
        tracing::info!("  {}:{}", node.name, node.version);
    }

    tracing::info!("分析完成");
}
