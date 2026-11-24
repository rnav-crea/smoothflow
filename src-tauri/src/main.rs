#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // NB-22: `restart_app` sets this marker before relaunching. The new process
    // can start before the old one releases the single-instance lock, and the
    // plugin would then classify it as a second instance and exit — leaving no
    // app running. Waiting here gives the old process time to be gone.
    if std::env::var_os("SMOOTHFLOW_RESTART").is_some() {
        std::env::remove_var("SMOOTHFLOW_RESTART");
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    smoothflow_lib::run()
}
