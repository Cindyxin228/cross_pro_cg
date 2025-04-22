mod dependency_analyzer;
use dependency_analyzer::DependencyAnalyzer;
use std::fs;
use std::path::Path;
use tracing_log::LogTracer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;

#[tokio::main]
async fn main() {
    // 手动删除日志文件
    let log_file_path = Path::new("logs/cross_pro_cg.log");
    if log_file_path.exists() {
        fs::remove_file(log_file_path).expect("无法删除旧日志文件");
    }

    let _guard = log_init();

    tracing::info!("开始分析依赖关系");
    let analyzer = DependencyAnalyzer::new("cindy", "crates_io_db");

    // 分析 ring crate 的依赖关系
    tracing::info!("分析 ring crate 的依赖关系");
    let leaf_nodes = analyzer
        .find_all_dependents(
            "crossbeam-channel",
            ">=0.5.11, <0.5.15",
            "crossbeam_channel::flavors::list::Channel::discard_all_messages",
        )
        .await;

    // 输出结果
    tracing::info!("找到的叶子节点数量: {}", leaf_nodes.len());
    tracing::info!("叶子节点列表:");
    for node in leaf_nodes {
        tracing::info!("  {}:{}", node.name, node.version);
    }

    tracing::info!("分析完成");
}

fn log_init() -> tracing_appender::non_blocking::WorkerGuard {
    LogTracer::builder()
        .init()
        .expect("Failed to initialize LogTracer");

    let std_layer = tracing_subscriber::fmt::layer()
        .with_level(true)
        .with_writer(std::io::stdout)
        .with_filter(LevelFilter::INFO);

    let file_appender = tracing_appender::rolling::daily("logs", "cross_pro_cg.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_level(true)
        .with_writer(non_blocking)
        .with_filter(LevelFilter::INFO);

    let collector = tracing_subscriber::registry()
        .with(std_layer)
        .with(file_layer);

    tracing::subscriber::set_global_default(collector).expect("Failed to set subscriber");

    guard
}
