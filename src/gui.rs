use std::sync::{mpsc, Arc};
use std::time::Duration;
use eframe::egui;
use tokio::runtime::Runtime;

use crate::config::Config;
use crate::engine::BackupEngine;
use crate::model::*;
use crate::progress::ProgressTracker;
use crate::store::sqlite::SqliteStore;
use crate::store::MetadataStore;
use crate::verifier::Verifier;

enum GuiMessage {
    RootsLoaded(Vec<BackupRoot>),
    TargetsLoaded(Vec<BackupTarget>),
    RootAdded,
    TargetAdded,
    BackupFinished(Result<(), String>),
    VerifyFinished(Result<(), String>),
    Error(String),
}

pub struct TmApp {
    config: Config,
    rt: Runtime,
    store: Arc<dyn MetadataStore>,
    progress: Arc<ProgressTracker>,

    source_is_remote: bool,
    source_path: String,
    source_label: String,
    target_path: String,
    target_subdir: String,
    verify_with_hash: bool,

    roots: Vec<BackupRoot>,
    targets: Vec<BackupTarget>,

    msg_tx: mpsc::Sender<GuiMessage>,
    msg_rx: mpsc::Receiver<GuiMessage>,

    backup_running: bool,
    error_msg: Option<String>,
    info_msg: Option<String>,
}

impl TmApp {
    pub fn new(cc: &eframe::CreationContext<'_>, config: Config) -> anyhow::Result<Self> {
        setup_fonts(&cc.egui_ctx);

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        let store: Arc<dyn MetadataStore> = {
            let store = rt.block_on(async {
                let store = SqliteStore::new(&config.database.url).await?;
                store.apply_migrations().await?;
                anyhow::Ok::<Arc<dyn MetadataStore>>(Arc::new(store))
            })?;
            store
        };

        let (msg_tx, msg_rx) = mpsc::channel();

        let app = Self {
            config,
            rt,
            store: store.clone(),
            progress: Arc::new(ProgressTracker::new()),
            source_is_remote: false,
            source_path: String::new(),
            source_label: String::new(),
            target_path: String::new(),
            target_subdir: "BACKUPDATABASE".to_string(),
            verify_with_hash: false,
            roots: Vec::new(),
            targets: Vec::new(),
            msg_tx,
            msg_rx,
            backup_running: false,
            error_msg: None,
            info_msg: None,
        };

        app.reload_roots();
        app.reload_targets();

        Ok(app)
    }

    fn reload_roots(&self) {
        let store = self.store.clone();
        let tx = self.msg_tx.clone();
        self.rt.spawn(async move {
            match store.load_backup_roots().await {
                Ok(roots) => { let _ = tx.send(GuiMessage::RootsLoaded(roots)); }
                Err(e) => { let _ = tx.send(GuiMessage::Error(e.to_string())); }
            }
        });
    }

    fn reload_targets(&self) {
        let store = self.store.clone();
        let tx = self.msg_tx.clone();
        self.rt.spawn(async move {
            match store.load_backup_targets().await {
                Ok(targets) => { let _ = tx.send(GuiMessage::TargetsLoaded(targets)); }
                Err(e) => { let _ = tx.send(GuiMessage::Error(e.to_string())); }
            }
        });
    }

    fn add_root(&self) {
        let store = self.store.clone();
        let tx = self.msg_tx.clone();
        let path = self.source_path.clone();
        let source_type = if self.source_is_remote { "remote" } else { "local" };
        let label = if self.source_label.is_empty() {
            None
        } else {
            Some(self.source_label.clone())
        };

        self.rt.spawn(async move {
            match store.add_backup_root(&path, source_type, label.as_deref()).await {
                Ok(_) => { let _ = tx.send(GuiMessage::RootAdded); }
                Err(e) => { let _ = tx.send(GuiMessage::Error(e.to_string())); }
            }
        });
    }

    fn add_target(&self) {
        let store = self.store.clone();
        let tx = self.msg_tx.clone();
        let path = self.target_path.clone();
        let subdir = self.target_subdir.clone();

        self.rt.spawn(async move {
            match store.add_backup_target(&path, &subdir, None).await {
                Ok(_) => { let _ = tx.send(GuiMessage::TargetAdded); }
                Err(e) => { let _ = tx.send(GuiMessage::Error(e.to_string())); }
            }
        });
    }

    fn start_backup(&mut self) {
        self.backup_running = true;
        self.error_msg = None;
        self.info_msg = None;

        let store = self.store.clone();
        let progress = self.progress.clone();
        let tx = self.msg_tx.clone();
        let hash_algo = self.config.hash_algo();
        let copy_concurrency = self.config.backup.copy_concurrency;
        let skip_hidden = self.config.backup.skip_hidden;

        progress.start();

        self.rt.spawn(async move {
            let engine = BackupEngine::new(store, hash_algo, copy_concurrency, skip_hidden)
                .with_progress(progress.clone());

            let result = engine.run().await;
            match result {
                Ok(()) => {
                    let files = progress.processed_files();
                    let bytes = progress.processed_bytes();
                    progress.complete(files, bytes);
                    let _ = tx.send(GuiMessage::BackupFinished(Ok(())));
                }
                Err(e) => {
                    progress.fail(e.to_string());
                    let _ = tx.send(GuiMessage::BackupFinished(Err(e.to_string())));
                }
            }
        });
    }

    fn start_verify(&mut self) {
        let store = self.store.clone();
        let tx = self.msg_tx.clone();
        let hash_algo = self.config.hash_algo();
        let with_hash = self.verify_with_hash;

        self.rt.spawn(async move {
            let verifier = Verifier::new(store, hash_algo);
            let result = verifier.check_data(with_hash).await;
            match result {
                Ok(()) => { let _ = tx.send(GuiMessage::VerifyFinished(Ok(()))); }
                Err(e) => { let _ = tx.send(GuiMessage::VerifyFinished(Err(e.to_string()))); }
            }
        });
    }

    fn process_messages(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                GuiMessage::RootsLoaded(roots) => self.roots = roots,
                GuiMessage::TargetsLoaded(targets) => self.targets = targets,
                GuiMessage::RootAdded => {
                    self.source_path.clear();
                    self.source_label.clear();
                    self.info_msg = Some("备份源已添加".to_string());
                    self.reload_roots();
                }
                GuiMessage::TargetAdded => {
                    self.target_path.clear();
                    self.info_msg = Some("目标盘已添加".to_string());
                    self.reload_targets();
                }
                GuiMessage::BackupFinished(result) => {
                    self.backup_running = false;
                    match result {
                        Ok(()) => self.info_msg = Some("备份完成".to_string()),
                        Err(e) => self.error_msg = Some(format!("备份失败: {}", e)),
                    }
                }
                GuiMessage::VerifyFinished(result) => match result {
                    Ok(()) => self.info_msg = Some("校验完成".to_string()),
                    Err(e) => self.error_msg = Some(format!("校验失败: {}", e)),
                },
                GuiMessage::Error(e) => self.error_msg = Some(e),
            }
        }
    }
}

impl eframe::App for TmApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.process_messages();

        let ctx = ui.ctx().clone();
        let need_repaint = self.backup_running || self.progress.is_running();
        if need_repaint {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            self.render_header(ui);
            self.render_messages(ui);
            self.render_source_section(ui);
            ui.separator();
            self.render_target_section(ui);
            ui.separator();
            self.render_actions(ui);
            ui.separator();
            self.render_progress(ui);
            ui.separator();
            self.render_data_lists(ui);
            ui.separator();
            self.render_logs(ui);
        });
    }
}

impl TmApp {
    fn render_header(&self, ui: &mut egui::Ui) {
        ui.heading("TM-RUST 备份系统");
        ui.label("支持局域网远程备份 · 多盘负载均衡 · 内容寻址存储");
        ui.add_space(4.0);
    }

    fn render_messages(&mut self, ui: &mut egui::Ui) {
        if let Some(ref err) = self.error_msg {
            ui.colored_label(egui::Color32::from_rgb(220, 80, 80), format!("⚠ {}", err));
        }
        if let Some(ref info) = self.info_msg {
            ui.colored_label(egui::Color32::from_rgb(80, 180, 80), format!("✓ {}", info));
        }
        ui.add_space(2.0);
    }

    fn render_source_section(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("备份源设置");
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.radio_value(&mut self.source_is_remote, false, "本地目录");
                ui.radio_value(&mut self.source_is_remote, true, "远程主机");
            });

            ui.horizontal(|ui| {
                let label = if self.source_is_remote { "地址:" } else { "路径:" };
                ui.label(label);
                ui.add(egui::TextEdit::singleline(&mut self.source_path).desired_width(400.0));

                if !self.source_is_remote {
                    if ui.button("浏览...").clicked() {
                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                            self.source_path = folder.to_string_lossy().to_string();
                        }
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("标签:");
                ui.add(egui::TextEdit::singleline(&mut self.source_label).desired_width(200.0));
                ui.add_space(10.0);
                if ui.button("添加备份源").clicked() && !self.source_path.is_empty() {
                    self.add_root();
                }
            });
        });
    }

    fn render_target_section(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("目标盘设置");
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("路径:");
                ui.add(egui::TextEdit::singleline(&mut self.target_path).desired_width(400.0));
                if ui.button("浏览...").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        self.target_path = folder.to_string_lossy().to_string();
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("子目录:");
                ui.add(egui::TextEdit::singleline(&mut self.target_subdir).desired_width(200.0));
                ui.add_space(10.0);
                if ui.button("添加目标盘").clicked() && !self.target_path.is_empty() {
                    self.add_target();
                }
            });
        });
    }

    fn render_actions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let backup_enabled = !self.backup_running
                && !self.roots.is_empty()
                && !self.targets.is_empty();
            if ui.add_enabled(backup_enabled, egui::Button::new("开始备份")).clicked() {
                self.start_backup();
            }

            ui.separator();

            ui.checkbox(&mut self.verify_with_hash, "校验时计算哈希");
            if ui.button("数据校验").clicked() && !self.targets.is_empty() {
                self.start_verify();
            }
        });
        ui.add_space(2.0);
    }

    fn render_progress(&self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("进度");
            ui.add_space(4.0);

            let percent = self.progress.progress_percent();
            let progress_bar = egui::ProgressBar::new(percent)
                .text(format!("{:.1}%  ({}/{})", percent * 100.0, self.progress.processed_files(), self.progress.total_files()));
            ui.add(progress_bar);

            ui.add_space(4.0);

            let current_root = self.progress.current_root();
            if !current_root.is_empty() {
                ui.label(format!("当前备份源: {}", current_root));
            }

            let current_file = self.progress.current_file();
            if !current_file.is_empty() {
                let display = if current_file.len() > 80 {
                    format!("...{}", &current_file[current_file.len() - 77..])
                } else {
                    current_file.clone()
                };
                ui.label(format!("当前文件: {}", display));
            }

            let bytes = self.progress.processed_bytes();
            if bytes > 0 {
                ui.label(format!(
                    "已写入: {:.2} MB ({:.2} GB)",
                    bytes as f64 / 1_048_576.0,
                    bytes as f64 / 1_073_741_824.0
                ));
            }

            let state = self.progress.state();
            match &state {
                crate::progress::BackupState::Idle => {
                    ui.colored_label(egui::Color32::from_gray(160), "待机");
                }
                crate::progress::BackupState::Running => {
                    ui.colored_label(egui::Color32::from_rgb(80, 140, 220), "备份进行中...");
                }
                crate::progress::BackupState::Completed { files, bytes } => {
                    ui.colored_label(
                        egui::Color32::from_rgb(80, 180, 80),
                        format!("完成: {} 文件, {:.2} MB", files, *bytes as f64 / 1_048_576.0),
                    );
                }
                crate::progress::BackupState::Failed(err) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 80, 80),
                        format!("失败: {}", err),
                    );
                }
            }
        });
    }

    fn render_data_lists(&self, ui: &mut egui::Ui) {
        ui.heading("已配置项");
        ui.add_space(4.0);

        ui.label(format!("备份源 ({}):", self.roots.len()));
        for r in &self.roots {
            ui.horizontal(|ui| {
                ui.label(format!("  #{} [{}] {}",
                    r.id, r.source_type, r.root_path));
                if let Some(ref label) = r.label {
                    ui.label(format!(" ({})", label));
                }
            });
        }

        ui.add_space(4.0);
        ui.label(format!("目标盘 ({}):", self.targets.len()));
        for t in &self.targets {
            let free = get_free_space(&t.target_path);
            ui.label(format!(
                "  #{} {} (子目录: {}, 剩余: {:.1} GB)",
                t.id, t.target_path, t.subdir_name,
                free as f64 / 1_073_741_824.0
            ));
        }
    }

    fn render_logs(&self, ui: &mut egui::Ui) {
        ui.heading("日志");
        ui.add_space(2.0);
        egui::ScrollArea::vertical()
            .max_height(150.0)
            .show(ui, |ui| {
                let logs = self.progress.logs();
                for line in &logs {
                    ui.label(line);
                }
            });
    }
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let font_paths = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyh.ttf",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "C:\\Windows\\Fonts\\simsun.ttc",
    ];

    for path in &font_paths {
        if let Ok(font_data) = std::fs::read(path) {
            fonts.font_data.insert(
                "cjk".to_owned(),
                Arc::new(egui::FontData::from_owned(font_data)),
            );
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.push("cjk".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.push("cjk".to_owned());
            }
            break;
        }
    }

    ctx.set_fonts(fonts);
}

pub fn run_gui(config: Config) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 760.0])
            .with_title("TM-RUST 备份系统"),
        ..Default::default()
    };

    eframe::run_native(
        "TM-RUST",
        options,
        Box::new(|cc| Ok(Box::new(TmApp::new(cc, config)?))),
    )
    .map_err(|e| anyhow::anyhow!("GUI 启动失败: {}", e))?;

    Ok(())
}

fn get_free_space(path: &str) -> u64 {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        let drive = if path.len() >= 2 && path.as_bytes()[1] == b':' {
            format!("{}\\", &path[..2])
        } else {
            path.to_string()
        };
        let wide: Vec<u16> = OsStr::new(&drive)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut avail: u64 = 0;
        let mut total: u64 = 0;
        let mut free: u64 = 0;
        extern "system" {
            fn GetDiskFreeSpaceExW(
                dir: *const u16,
                avail: *mut u64,
                total: *mut u64,
                free: *mut u64,
            ) -> i32;
        }
        let ret = unsafe {
            GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free)
        };
        if ret != 0 { avail } else { 0 }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        0
    }
}
