pub mod cli_export;
pub mod overlay;
pub mod quota;
pub mod runtime;
pub mod startup;
pub mod tracker;
pub mod tray;

pub static REFRESH_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// トレイクリックや状態変化時にクォータ更新を即時要求
pub fn request_refresh() {
    REFRESH_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// 更新要求フラグを確認
pub fn is_refresh_requested() -> bool {
    REFRESH_REQUESTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// 更新要求フラグをリセット
pub fn clear_refresh_request() {
    REFRESH_REQUESTED.store(false, std::sync::atomic::Ordering::SeqCst);
}
