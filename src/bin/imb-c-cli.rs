//! 主控台子系統入口；不初始化 Slint。
#[path = "../cli.rs"]
mod cli;
#[allow(dead_code)]
#[path = "../engine.rs"]
mod engine;
#[path = "../iat.rs"]
mod iat;
#[path = "../imports.rs"]
mod imports;
fn main() {
    if engine::initialize_dll_search().is_err() {
        std::process::exit(2);
    }
    std::process::exit(cli::run(std::env::args_os().skip(1).collect()));
}
