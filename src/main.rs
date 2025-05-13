mod database;
mod dependency_analyzer;
mod krate;
mod logger;

use dependency_analyzer::DependencyAnalyzer;
use std::fs;
use std::path::Path;

#[tokio::main]
async fn main() {
    let crate_name = "crossbeam-channel";
    let version_range = ">0.5.11, <0.5.15";
    let target_function_path = "crossbeam_channel::flavors::list::Channel::drop";
    let log_file_path = Path::new("logs/cross_pro_cg.log");

    dotenv::dotenv().ok();
    let _guard = logger::log_init();
    if log_file_path.exists() {
        fs::remove_file(log_file_path).expect("无法删除旧日志文件");
    }

    tracing::info!("开始分析依赖关系");
    let analyzer = DependencyAnalyzer::new().await.unwrap();
    analyzer
        .analyze(crate_name, version_range, target_function_path)
        .await
        .unwrap();

    tracing::info!("分析完成");
}
