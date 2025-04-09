mod dependency_analyzer;
use dependency_analyzer::DependencyAnalyzer;

use std::fs::File;
use std::io::Write;
use tracing::{info, warn, error, debug, trace};
use tracing_subscriber::EnvFilter;

fn main() {
    // 初始化tracing
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()))
        .init();
    
    info!("开始分析依赖关系");
    let mut analyzer = DependencyAnalyzer::new("cindy", "crates_io_db");
    
    info!("测试 serde 的版本范围 ^1.0 的叶子节点");
    let leaf_nodes = analyzer.find_all_dependents("ring", "<0.17.12", "ring::aead::Nonce::assume_unique_for_key");
    info!("找到的叶子节点数量: {}", leaf_nodes.len());
    info!("叶子节点列表:");
    for node in leaf_nodes {
        info!("  {}:{}", node.name, node.version);
    }
    
    info!("分析完成");
}