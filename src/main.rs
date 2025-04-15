mod dependency_analyzer;
use dependency_analyzer::DependencyAnalyzer;

use tracing::{info, warn, error, debug, trace};
use tracing_subscriber::{fmt, EnvFilter};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use std::fs;
use std::path::Path;

fn main() {
    // 手动删除日志文件
    let log_file_path = Path::new("logs/cross_pro_cg.log");
    if log_file_path.exists() {
        fs::remove_file(log_file_path).expect("无法删除旧日志文件");
    }
    
    // 创建文件输出
    let file_appender = RollingFileAppender::new(
        Rotation::NEVER,
        "logs",
        "cross_pro_cg.log",
    );
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    
    // 初始化tracing，同时输出到终端和文件
    tracing_subscriber::fmt()
        .with_env_filter("debug")
        .with_writer(non_blocking)
        .with_ansi(false)  // 禁用ANSI颜色代码，因为文件不需要
        .init();
    
    info!("开始分析依赖关系");
    let mut analyzer = DependencyAnalyzer::new("cindy", "crates_io_db");
    
    // 分析 ring crate 的依赖关系
    info!("分析 ring crate 的依赖关系");
    let leaf_nodes = analyzer.find_all_dependents("crossbeam-channel", ">=0.5.11, <0.5.15", "crossbeam_channel::flavors::list::Channel::discard_all_messages");
    
    // 输出结果
    info!("找到的叶子节点数量: {}", leaf_nodes.len());
    info!("叶子节点列表:");
    for node in leaf_nodes {
        info!("  {}:{}", node.name, node.version);
    }
    
    info!("分析完成");
}