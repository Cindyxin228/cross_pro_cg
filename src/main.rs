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

    // 分析 crossbeam-channel crate 的依赖关系
    tracing::info!("分析 crossbeam-channel crate 的依赖关系");
    let dependency_tree = analyzer
        .build_dependency_tree(
            "crossbeam-channel",
            ">=0.5.11, <0.5.15",
            "crossbeam_channel::flavors::list::Channel::discard_all_messages",
        )
        .await
        .unwrap();

    // 输出结果
    tracing::info!("构建的依赖树根节点有 {} 个直接子节点", dependency_tree.dependents().len());
    tracing::info!("依赖树结构:");
    for child in dependency_tree.dependents() {
        tracing::info!("  {}:{}", child.name(), child.version());
    }

    tracing::info!("分析完成");
}
