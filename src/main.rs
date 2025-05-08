mod database;
mod dependency_analyzer;
mod krate;
mod logger;

use dependency_analyzer::DependencyAnalyzer;
use std::fs;
use std::path::Path;
use std::collections::VecDeque;

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

    // 1. 解析版本范围，初始化根节点和 dependents
    let version_req = analyzer.parse_version_requirement(">=0.5.11, <0.5.15").unwrap();
    let mut root = analyzer.create_root_node();
    let mut _tmp_queue = VecDeque::new();
    analyzer.initialize_root_children(&mut root, "crossbeam-channel", &version_req, &mut _tmp_queue).await.unwrap();

    // 2. 调用 BFS 分析
    analyzer
        .bfs_dependency_analysis(
            &root,
            "crossbeam_channel::flavors::list::Channel::discard_all_messages",
            32, // 并发数
        )
        .await
        .unwrap();

    tracing::info!("分析完成");
}
