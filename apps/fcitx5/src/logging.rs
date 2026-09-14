//! 日志：默认写 stderr；设了 `QINGJIAN_LOG_DIR` 就按天写文件（与 CLI / 其它壳一致）。

use std::path::Path;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// 初始化全局日志。重复调用是空操作。返回的 guard 必须活到进程结束，否则文件日志会停。
pub fn init() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    match std::env::var_os("QINGJIAN_LOG_DIR") {
        Some(dir) if !dir.is_empty() => {
            let appender = tracing_appender::rolling::daily(Path::new(&dir), "fcitx5.log");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer().with_writer(writer))
                .try_init()
                .ok();
            Some(guard)
        }
        _ => {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
                .try_init()
                .ok();
            None
        }
    }
}
