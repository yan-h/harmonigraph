use egui_baseview::{
    EguiWindow, EguiWindowSettings, Frame,
    baseview::{
        self, Event, EventStatus, HandlerError, Window, WindowContext, WindowHandler,
        WindowSettings, WindowSize, dpi::LogicalSize,
    },
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
struct Host {
    did_resize: std::cell::Cell<bool>,
    window: WindowContext,
    start: Instant,
    child: std::cell::RefCell<Option<Window>>,
}
impl WindowHandler for Host {
    fn on_frame(&self) -> Result<(), HandlerError> {
        if self.start.elapsed() > Duration::from_secs(9) && !self.did_resize.replace(true) {
            if let Some(child) = self.child.borrow().as_ref() {
                println!("HOST_RESIZE physical=1080x600");
                child.resize(baseview::dpi::PhysicalSize::new(1080, 600)).unwrap();
            }
        }
        if self.start.elapsed() > Duration::from_secs(12) {
            if let Some(child) = self.child.borrow_mut().take() {
                child.close();
            }
            self.window.request_close();
        }
        Ok(())
    }
    fn resized(&self, _: WindowSize) -> Result<(), HandlerError> {
        Ok(())
    }
    fn on_event(&self, _: Event) -> EventStatus {
        EventStatus::Ignored
    }
}
struct Probe {
    frames: Arc<AtomicUsize>,
    image: Arc<Mutex<Option<egui::TextureHandle>>>,
    start: Instant,
    resize_sent: bool,
}
impl egui_baseview::App for Probe {
    fn build(&mut self, ctx: egui::Context, frame: &mut Frame) -> Result<(), HandlerError> {
        println!("BUILD native={:?}", frame.baseview_window().size());
        let frames = self.frames.clone();
        let image = self.image.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            let before = frames.load(Ordering::SeqCst);
            let mut fonts = egui::FontDefinitions::default();
            fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().reverse();
            ctx.set_fonts(fonts);
            *image.lock().unwrap() = Some(ctx.load_texture(
                "idle-image",
                egui::ColorImage::filled([16, 16], egui::Color32::RED),
                egui::TextureOptions::NEAREST,
            ));
            println!("MUTATED idle font/image before_frames={before}");
            std::thread::sleep(Duration::from_secs(2));
            println!("UNWOKEN after_frames={}", frames.load(Ordering::SeqCst));
            ctx.request_repaint();
            println!("EXPLICIT_WAKE");
            std::thread::sleep(Duration::from_secs(2));
            println!("WOKEN after_frames={}", frames.load(Ordering::SeqCst));
            image.lock().unwrap().as_mut().unwrap().set(
                egui::ColorImage::filled([16, 16], egui::Color32::GREEN),
                egui::TextureOptions::NEAREST,
            );
            ctx.request_repaint_after(Duration::from_millis(500));
            println!("DELAYED_WAKE 500ms");
        });
        Ok(())
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        let n = self.frames.fetch_add(1, Ordering::SeqCst) + 1;
        println!(
            "UI {n} t={:.3} native={:?} screen={:?} ppp={}",
            self.start.elapsed().as_secs_f64(),
            frame.baseview_window().size(),
            ui.ctx().content_rect(),
            ui.ctx().pixels_per_point()
        );
        if !self.resize_sent {
            self.resize_sent = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(320., 180.)));
            println!("RESIZE request 320x180 egui points zoom=1.5");
        }
        ui.label("Font atlas update proof ABC 123");
        if let Some(image) = self.image.lock().unwrap().as_ref() {
            ui.image(image);
        }
    }
    fn resized(&mut self, size: WindowSize) {
        println!("RESIZED reported={size:?}");
    }
}
fn main() {
    unsafe { baseview::assume_standalone_in_process() };
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(15));
        println!("PROBE_WATCHDOG_END");
        std::process::exit(0)
    });
    Window::create(
        WindowSettings::new()
            .with_title("GUI feasibility temporary host")
            .with_size(LogicalSize::new(700., 500.)),
        |host| {
            let child = EguiWindow::create(
                EguiWindowSettings::new()
                    .with_parent(&host)
                    .with_size(LogicalSize::new(300., 160.))
                    .with_zoom_factor(1.5),
                Probe {
                    frames: Arc::new(AtomicUsize::new(0)),
                    image: Arc::new(Mutex::new(None)),
                    start: Instant::now(),
                    resize_sent: false,
                },
            )
            .unwrap();
            child.show().unwrap();
            Ok(Host {
                did_resize: std::cell::Cell::new(false),
                window: host,
                start: Instant::now(),
                child: std::cell::RefCell::new(Some(child)),
            })
        },
    )
    .unwrap()
    .run_until_closed()
    .unwrap();
}
