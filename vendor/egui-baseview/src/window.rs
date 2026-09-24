use std::sync::Arc;
use std::time::{Duration, Instant};

use baseview::{
    Event, EventStatus, PhySize, Size, Window, WindowHandle, WindowHandler, WindowOpenOptions,
    WindowScalePolicy,
};
use copypasta::ClipboardProvider;
use egui::{Pos2, Rect, Rgba, ViewportCommand, pos2, vec2};
use keyboard_types::Modifiers;
use raw_window_handle::HasRawWindowHandle;

use crate::{GraphicsConfig, renderer::Renderer};

#[cfg(feature = "nice-log")]
use nice_plug_core::{nice_error as error, nice_warn as warn};

#[cfg(all(feature = "tracing", not(feature = "nice-log")))]
use tracing::{error, warn};

/// Somewhere to ask, once a frame, whether this window has been resized from
/// OUTSIDE — a plugin host dragging the border of the window the view is
/// parented into, which on macOS arrives as no event at all.
///
/// Answered in logical size, since the window owns the scale factor.
///
/// The point of it is WHEN it is asked: before the frame's input is built. A
/// host resize applied later — from inside the update closure, which is where
/// a plugin would otherwise have to do it — lays the frame out at the size the
/// window has stopped being and presents it into a surface that has already
/// changed, so every frame of a drag is one frame of content that does not
/// fit.
#[derive(Clone)]
pub struct SizeSource(Arc<dyn Fn() -> Option<Size> + Send + Sync>);

impl SizeSource {
    pub fn new(source: impl Fn() -> Option<Size> + Send + Sync + 'static) -> Self {
        SizeSource(Arc::new(source))
    }

    fn take(&self) -> Option<Size> {
        (self.0)()
    }
}

impl std::fmt::Debug for SizeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SizeSource")
    }
}

#[derive(Debug, Clone)]
pub struct EguiWindowSettings {
    pub title: String,

    /// The logical size of the window
    ///
    /// These dimensions will be scaled by the scaling policy specified in `scale`. Mouse
    /// position will be passed back as logical coordinates.
    pub logical_size: Size,

    /// The dpi scaling policy
    pub scale_policy: WindowScalePolicy,

    pub graphics: GraphicsConfig,

    /// Seconds between frame-timer ticks the window opens with. See
    /// [`baseview::WindowOpenOptions::frame_interval`]; change it later
    /// through [`Queue::set_frame_interval`].
    pub frame_interval: f64,

    /// Where a size given to this window from outside arrives, if anywhere.
    /// See [`SizeSource`].
    pub size_source: Option<SizeSource>,
}

impl EguiWindowSettings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_tile(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn with_logical_size(mut self, size: Size) -> Self {
        self.logical_size = size;
        self
    }

    pub fn with_scale_policy(mut self, scale_policy: WindowScalePolicy) -> Self {
        self.scale_policy = scale_policy;
        self
    }

    pub fn with_graphics_config(mut self, config: GraphicsConfig) -> Self {
        self.graphics = config;
        self
    }

    pub fn with_frame_interval(mut self, frame_interval: f64) -> Self {
        self.frame_interval = frame_interval;
        self
    }

    pub fn with_size_source(mut self, source: SizeSource) -> Self {
        self.size_source = Some(source);
        self
    }
}

impl Default for EguiWindowSettings {
    fn default() -> Self {
        Self {
            title: String::new(),
            logical_size: Size {
                width: 300.0,
                height: 200.0,
            },
            scale_policy: WindowScalePolicy::default(),
            graphics: GraphicsConfig::default(),
            frame_interval: baseview::DEFAULT_FRAME_INTERVAL,
            size_source: None,
        }
    }
}

pub struct Queue<'a> {
    bg_color: &'a mut Rgba,
    close_requested: &'a mut bool,
    physical_size: &'a mut PhySize,
    key_capture: &'a mut KeyCapture,
    frame_interval: &'a mut Option<f64>,
    display_max_fps: Option<f64>,
    tess_ms: f32,
    draw_gpu_ms: f32,
    acquire_ms: f32,
    tick_ms: f32,
    render_ms: f32,
    upload_ms: f32,
    ubuf_ms: f32,
    texture_ms: f32,
    encode_ms: f32,
    submit_ms: f32,
    prims: u32,
    verts: u32,
}

impl<'a> Queue<'a> {
    pub(crate) fn new(
        bg_color: &'a mut Rgba,
        close_requested: &'a mut bool,
        physical_size: &'a mut PhySize,
        key_capture: &'a mut KeyCapture,
        frame_interval: &'a mut Option<f64>,
        display_max_fps: Option<f64>,
        tess_ms: f32,
        draw_gpu_ms: f32,
        acquire_ms: f32,
        tick_ms: f32,
        render_ms: f32,
        upload_ms: f32,
        ubuf_ms: f32,
        texture_ms: f32,
        encode_ms: f32,
        submit_ms: f32,
        prims: u32,
        verts: u32,
    ) -> Self {
        Self {
            bg_color,
            //renderer,
            //repaint_requested,
            close_requested,
            physical_size,
            key_capture,
            frame_interval,
            display_max_fps,
            tess_ms,
            draw_gpu_ms,
            acquire_ms,
            tick_ms,
            render_ms,
            upload_ms,
            ubuf_ms,
            texture_ms,
            encode_ms,
            submit_ms,
            prims,
            verts,
        }
    }

    /// How many primitives and vertices the previous frame uploaded.
    pub fn prims(&self) -> u32 {
        self.prims
    }

    pub fn verts(&self) -> u32 {
        self.verts
    }

    /// Of the uploads, the TEXTURE half.
    pub fn texture_ms(&self) -> f32 {
        self.texture_ms
    }

    /// The renderer's stages, in milliseconds: uploads (which is also where
    /// paint callbacks `prepare`), encoding egui's draw calls, and
    /// finish + submit + present.
    pub fn upload_ms(&self) -> f32 {
        self.upload_ms
    }

    /// Of that, `update_buffers` itself. See the renderer's accessor for why
    /// the two are not the same number.
    pub fn ubuf_ms(&self) -> f32 {
        self.ubuf_ms
    }

    pub fn encode_ms(&self) -> f32 {
        self.encode_ms
    }

    pub fn submit_ms(&self) -> f32 {
        self.submit_ms
    }

    /// Milliseconds the previous frame spent inside the renderer: tessellate,
    /// buffer and texture uploads, encoding egui's pass, acquire, submit and
    /// present. `tick_ms` minus this is the egui half.
    pub fn render_ms(&self) -> f32 {
        self.render_ms
    }

    /// Milliseconds the previous frame callback took, end to end.
    ///
    /// Every other reading is a STAGE; this is the whole thing, and it is what
    /// the frame timer has to fit inside its period. Compared against the
    /// interval between frames it answers the only question the stages cannot:
    /// whether a long frame was slow, or simply late being asked for.
    pub fn tick_ms(&self) -> f32 {
        self.tick_ms
    }

    /// Milliseconds the previous frame blocked waiting for the surface.
    ///
    /// Large here with every cost row small means the frame is not slow, it is
    /// early — the display, not the work, is setting the pace.
    pub fn acquire_ms(&self) -> f32 {
        self.acquire_ms
    }

    /// Milliseconds the GPU spent from callback preparation through egui's
    /// composite a few frames ago,
    /// or 0 where the device can't measure it.
    ///
    /// The lattice's 3D time is included here and also shown separately in
    /// the overlay. Queue uploads and callback-owned command buffers are
    /// outside this bracket.
    pub fn draw_gpu_ms(&self) -> f32 {
        self.draw_gpu_ms
    }

    /// Milliseconds the PREVIOUS frame spent tessellating egui's shapes.
    ///
    /// Between the app's own frame time and the GPU's, this is the step that
    /// is otherwise invisible: shapes are cheap to append and expensive to
    /// turn into triangles, and the two happen in different places.
    pub fn tess_ms(&self) -> f32 {
        self.tess_ms
    }

    /// The highest refresh rate the display showing this window can present
    /// at, in Hz, or `None` where the platform won't say.
    ///
    /// Re-read every frame, so it follows the window to another monitor
    /// rather than being fixed at whatever it was when the window opened.
    pub fn display_max_fps(&self) -> Option<f64> {
        self.display_max_fps
    }

    /// Re-arm the window's frame timer at `interval` seconds, bounding how
    /// often the app is asked to draw from here on.
    ///
    /// Applied once the frame returns (the window isn't reachable from
    /// inside), so calling it repeatedly within one frame keeps only the last
    /// value. Cheap to call every frame with an unchanged interval — the
    /// caller-side comparison is left to the app, which knows what it set.
    pub fn set_frame_interval(&mut self, interval: f64) {
        *self.frame_interval = Some(interval);
    }

    /// Set the background color.
    pub fn bg_color(&mut self, bg_color: Rgba) {
        *self.bg_color = bg_color;
    }

    /// Set size of the window.
    pub fn resize(&mut self, physical_size: PhySize) {
        *self.physical_size = physical_size;
    }

    /// Close the window.
    pub fn close_window(&mut self) {
        *self.close_requested = true;
    }

    /// Set how to handle capturing key events from the host.
    pub fn set_key_capture(&mut self, key_capture: KeyCapture) {
        *self.key_capture = key_capture;
    }
}

/// Describes how to handle capturing key events from the host.
#[derive(Default, Debug, Clone, PartialEq)]
pub enum KeyCapture {
    #[default]
    /// All keys will be captured from the host.
    CaptureAll,
    /// No keys will be captured from the host.
    IgnoreAll,
    /// Only the given keys will be captured from the host.
    CaptureKeys(Vec<keyboard_types::Key>),
    /// All keys except the given ones will be captured from the host.
    IgnoreKeys(Vec<keyboard_types::Key>),
}

/// Handles an egui-baseview application
pub struct EguiWindow<State, U>
where
    State: 'static + Send,
    U: FnMut(&mut egui::Ui, &mut Queue, &mut State),
    U: 'static + Send,
{
    user_state: Option<State>,
    user_update: U,

    egui_ctx: egui::Context,
    viewport_id: egui::ViewportId,
    start_time: Instant,
    egui_input: egui::RawInput,
    /// Where the pointer was last seen, which is where a button event happens:
    /// baseview reports a press and a release without one. `None` only before
    /// the first move — and it is not cleared when the pointer leaves, because
    /// a release still lands where the pointer was (see `CursorLeft`).
    pointer_pos_in_points: Option<egui::Pos2>,
    current_cursor_icon: baseview::MouseCursor,

    renderer: Renderer,

    clipboard_ctx: Option<copypasta::ClipboardContext>,

    physical_size: PhySize,
    /// See [`SizeSource`]: polled at the top of every frame, before the input
    /// that the frame is laid out from is built.
    size_source: Option<SizeSource>,
    scale_policy: WindowScalePolicy,
    pixels_per_point: f32,
    points_per_pixel: f32,
    bg_color: Rgba,
    close_requested: bool,
    repaint_after: Option<Instant>,
    /// How many frames in a row the surface has refused to present. Reset by
    /// the first one that lands. See [`REFUSALS_BEFORE_BACKOFF`].
    refused_presents: u32,
    /// When the next frame offered to a refusing surface is due, or `None`
    /// while the surface is still being trusted to take one every tick.
    ///
    /// Separate from `repaint_after` because it paces something else:
    /// `repaint_after` is what the UI asked for, this is what the surface has
    /// shown it will accept. See [`wants_render`].
    retry_after: Option<Instant>,
    key_capture: KeyCapture,
    /// Tessellation time from the PREVIOUS frame — it is measured inside
    /// `render`, which runs after the update closure, so the closure can only
    /// ever be handed the last one. One frame stale, like every other
    /// after-the-fact measurement here.
    tess_ms: f32,
    /// Likewise for GPU drawing, which lags further still — the timestamps
    /// have to come back from the GPU.
    draw_gpu_ms: f32,
    /// Likewise for the surface wait.
    acquire_ms: f32,
    /// The whole previous callback, end to end.
    tick_ms: f32,
    /// The renderer half of it, and its stages.
    render_ms: f32,
    upload_ms: f32,
    ubuf_ms: f32,
    texture_ms: f32,
    encode_ms: f32,
    submit_ms: f32,
    prims: u32,
    verts: u32,
}

impl<State, U> EguiWindow<State, U>
where
    State: 'static + Send,
    U: FnMut(&mut egui::Ui, &mut Queue, &mut State),
    U: 'static + Send,
{
    fn new<B>(
        window: &mut baseview::Window<'_>,
        settings: EguiWindowSettings,
        mut build: B,
        update: U,
        mut state: State,
    ) -> EguiWindow<State, U>
    where
        B: FnMut(&egui::Context, &mut Queue, &mut State),
        B: 'static + Send,
    {
        let renderer = Renderer::new(window, settings.graphics).unwrap_or_else(|err| {
            // TODO: better error log and not panicking, but that's gonna require baseview changes
            error!("oops! the gpu backend couldn't initialize! \n {err}");
            panic!("gpu backend failed to initialize: \n {err}")
        });
        let egui_ctx = egui::Context::default();

        // Assume scale for now until there is an event with a new one.
        let pixels_per_point = match settings.scale_policy {
            WindowScalePolicy::ScaleFactor(scale) => scale,
            WindowScalePolicy::SystemScaleFactor => 1.0,
        } as f32;
        let points_per_pixel = pixels_per_point.recip();

        let screen_rect = Rect::from_min_size(
            Pos2::new(0f32, 0f32),
            vec2(
                settings.logical_size.width as f32,
                settings.logical_size.height as f32,
            ),
        );

        let viewport_info = egui::ViewportInfo {
            parent: None,
            title: Some(settings.title),
            native_pixels_per_point: Some(pixels_per_point),
            focused: Some(true),
            inner_rect: Some(screen_rect),
            ..Default::default()
        };
        let viewport_id = egui::ViewportId::default();

        let mut egui_input = egui::RawInput {
            max_texture_side: Some(renderer.font_atlas_max_texture_side()),
            screen_rect: Some(screen_rect),
            ..Default::default()
        };
        let _ = egui_input.viewports.insert(viewport_id, viewport_info);

        let mut physical_size = PhySize {
            width: (settings.logical_size.width * pixels_per_point as f64).round() as u32,
            height: (settings.logical_size.height * pixels_per_point as f64).round() as u32,
        };

        let mut bg_color = Rgba::BLACK;
        let mut close_requested = false;
        let old_physical_size = physical_size;
        let mut key_capture = KeyCapture::default();
        let mut frame_interval = None;
        let mut queue = Queue::new(
            &mut bg_color,
            &mut close_requested,
            &mut physical_size,
            &mut key_capture,
            &mut frame_interval,
            window.display_max_fps(),
            // No frame has been measured yet: tess, draw gpu, acquire, tick,
            // render, upload, ubuf, texture, encode, submit, then the two
            // geometry counts.
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0,
            0,
        );
        (build)(&egui_ctx, &mut queue, &mut state);
        if let Some(interval) = frame_interval {
            window.set_frame_interval(interval);
        }

        if physical_size != old_physical_size {
            // `physical_size` is in physical pixels, but `Window::resize()`
            // takes logical points; convert or the window ends up
            // `pixels_per_point` times too large on scaled displays.
            window.resize(baseview::Size {
                width: physical_size.width as f64 * points_per_pixel as f64,
                height: physical_size.height as f64 * points_per_pixel as f64,
            });
        }

        let clipboard_ctx = match copypasta::ClipboardContext::new() {
            Ok(clipboard_ctx) => Some(clipboard_ctx),
            Err(e) => {
                error!("Failed to initialize clipboard: {}", e);
                None
            }
        };

        let start_time = Instant::now();

        Self {
            user_state: Some(state),
            user_update: update,

            egui_ctx,
            viewport_id,
            start_time,
            egui_input,
            pointer_pos_in_points: None,
            current_cursor_icon: baseview::MouseCursor::Default,

            renderer,

            clipboard_ctx,

            physical_size,
            size_source: settings.size_source.clone(),
            pixels_per_point,
            points_per_pixel,
            scale_policy: settings.scale_policy,
            bg_color,
            close_requested,
            repaint_after: Some(start_time),
            refused_presents: 0,
            retry_after: None,
            key_capture,
            tess_ms: 0.0,
            draw_gpu_ms: 0.0,
            acquire_ms: 0.0,
            tick_ms: 0.0,
            render_ms: 0.0,
            upload_ms: 0.0,
            ubuf_ms: 0.0,
            texture_ms: 0.0,
            encode_ms: 0.0,
            prims: 0,
            verts: 0,
            submit_ms: 0.0,
        }
    }

    /// Open a new child window.
    ///
    /// * `parent` - The parent window.
    /// * `settings` - The settings of the window.
    /// * `state` - The initial state of your application.
    /// * `build` - Called once before the first frame. Allows you to do setup code and to
    ///   call `ctx.set_fonts()`. Optional.
    /// * `update` - Called before each frame. Here you should update the state of your
    ///   application and build the UI.
    pub fn open_parented<P, B>(
        parent: &P,
        settings: EguiWindowSettings,
        state: State,
        build: B,
        update: U,
    ) -> WindowHandle
    where
        P: HasRawWindowHandle,
        B: FnMut(&egui::Context, &mut Queue, &mut State),
        B: 'static + Send,
    {
        Window::open_parented(
            parent,
            #[allow(clippy::needless_update)]
            WindowOpenOptions {
                title: settings.title.clone(),
                size: settings.logical_size,
                scale: settings.scale_policy,
                frame_interval: settings.frame_interval,
                #[cfg(feature = "opengl")]
                gl_config: Some(settings.graphics.gl_config.clone()),
                ..Default::default()
            },
            move |window: &mut baseview::Window<'_>| -> EguiWindow<State, U> {
                EguiWindow::new(window, settings, build, update, state)
            },
        )
    }

    /// Open a new window that blocks the current thread until the window is destroyed.
    ///
    /// * `settings` - The settings of the window.
    /// * `state` - The initial state of your application.
    /// * `build` - Called once before the first frame. Allows you to do setup code and to
    ///   call `ctx.set_fonts()`. Optional.
    /// * `update` - Called before each frame. Here you should update the state of your
    ///   application and build the UI.
    pub fn open_blocking<B>(settings: EguiWindowSettings, state: State, build: B, update: U)
    where
        B: FnMut(&egui::Context, &mut Queue, &mut State),
        B: 'static + Send,
    {
        Window::open_blocking(
            #[allow(clippy::needless_update)]
            WindowOpenOptions {
                title: settings.title.clone(),
                size: settings.logical_size,
                scale: settings.scale_policy,
                frame_interval: settings.frame_interval,
                #[cfg(feature = "opengl")]
                gl_config: Some(settings.graphics.gl_config.clone()),
                ..Default::default()
            },
            move |window: &mut baseview::Window<'_>| -> EguiWindow<State, U> {
                EguiWindow::new(window, settings, build, update, state)
            },
        )
    }

    /// Replace the pressed key modifiers with the set an event has sent, all of
    /// them — see `translate_modifiers` for why a partial update leaves the
    /// wheel dead.
    fn update_modifiers(&mut self, modifiers: &Modifiers) {
        self.egui_input.modifiers = crate::translate::translate_modifiers(*modifiers);
    }
}

/// Consecutive refused presents before the loop stops offering one every tick.
///
/// Small on purpose. The refusals this rides through are the transient ones —
/// a surface going Outdated or Lost while a resize is in flight, where
/// retrying on the very next tick is what keeps the drag smooth and is the
/// behaviour [`SURFACE_RETRY`] must not slow down. Refusal that outlasts a
/// few ticks is not transient, and that is what the backoff is for.
const REFUSALS_BEFORE_BACKOFF: u32 = 3;

/// How often a surface that keeps refusing is offered another frame.
///
/// Two things set it. Below the GPU's own frame time it stops bounding
/// anything, since the whole point is to submit slower than the device
/// drains; a hidden editor costs 4 unpresented frames a second here against
/// the 60-144 it used to. Above a few hundred milliseconds it starts being
/// visible in the case the poll exists for — a surface that has quietly
/// become available again without anything saying so — where this interval is
/// how long the window stays frozen.
const SURFACE_RETRY: Duration = Duration::from_millis(250);

/// Whether this tick paints, or stops after the egui pass it has already run.
///
/// Split out of the frame because it is the whole of the decision and the only
/// part of it a test can reach: the tick around it needs a live window, a
/// surface and a device.
///
/// `retry_after` is the term that is not about what the UI wants. A surface
/// that refuses to present — which is what an occluded window does, on every
/// tick, for as long as the editor is hidden behind something — makes the
/// frame one nobody sees. Drawing it anyway is not merely waste: the acquire
/// is ALSO where a presenting frame waits for vsync,
/// and that wait is the only backpressure in this loop. Refused immediately
/// instead of waited on, it leaves the tick free to encode and submit a whole
/// scene every timer tick while the GPU is still working through the last one.
/// wgpu reclaims a frame's staged uploads when its submission COMPLETES — the
/// poll at the end of `Queue::submit` is non-blocking, so it frees only what
/// has already landed — so submitting faster than the device drains grows the
/// in-flight set without bound: gigabytes within a minute of switching away
/// from the host, handed back all at once when the window comes forward and
/// vsync starts pacing again. Submitting slower than the device drains is what
/// bounds it. The flush in the renderer's `Occluded` arm is a different fix,
/// and only stops a frame that DID draw from stranding its uploads.
///
/// Keyed on the refusals themselves rather than on the occlusion event,
/// because the event decides nothing here and can disagree. wgpu-hal reads the
/// occlusion state by walking its own layer up to a delegate's window, while
/// baseview reads the view's; `Occluded(false)` can be lost or filtered
/// outright — a view momentarily without a window, a host reparenting the
/// plugin — and a window that never paints again is a far worse failure than
/// the memory this saves. The acquire is the one authority both agree on, the
/// same reasoning the renderer's `before_present` uses to un-hide the layer.
/// So a refusing surface is still offered a frame every [`SURFACE_RETRY`], and
/// the first one that presents clears the backoff.
///
/// Texture deltas are the exception, and they come first. They are uploaded
/// only inside `render()`, and a frame carrying one that is skipped drops it
/// permanently, leaving egui's glyph coordinates pointing into a stale atlas
/// (scrambled text). Honouring them during a backoff costs nothing that
/// repeats: consuming the delta is what clears it, so each one buys a single
/// frame rather than re-arming the loop this exists to stop.
fn wants_render(
    has_texture_updates: bool,
    repaint_after: Option<Instant>,
    retry_after: Option<Instant>,
    repaint_delay: Duration,
    now: Instant,
) -> bool {
    if has_texture_updates {
        return true;
    }
    if let Some(t) = retry_after {
        // Deliberately not `repaint_delay`: the UI asks for a zero-delay
        // repaint on every animating frame, and honouring that is the spin
        // this exists to stop. A refusing surface is paced by its own clock
        // and by nothing the UI has to say about it.
        return now >= t;
    }
    match repaint_after {
        Some(t) => now >= t || repaint_delay.is_zero(),
        None => repaint_delay.is_zero(),
    }
}

/// What a finished render leaves behind for [`wants_render`] to read next
/// tick: how many presents have now been refused in a row, and when to offer
/// the surface another frame.
///
/// The other half of the decision, and split out for the same reason — a
/// refusal is only reachable through a real surface, and the counter is the
/// whole of what separates a transient one from occlusion.
fn after_render(presented: bool, refused: u32, now: Instant) -> (u32, Option<Instant>) {
    if presented {
        return (0, None);
    }
    let refused = refused.saturating_add(1);
    let retry =
        (refused >= REFUSALS_BEFORE_BACKOFF).then(|| now.checked_add(SURFACE_RETRY)).flatten();
    (refused, retry)
}

#[cfg(test)]
mod wants_render_tests {
    use super::*;

    /// The shape the plugin's UI actually asks for while anything is moving:
    /// `request_repaint()`, which is a repaint delay of zero, and a deadline
    /// already behind us. Under it a visible window paints every tick.
    fn animating(now: Instant) -> (Option<Instant>, Duration) {
        (now.checked_sub(Duration::from_millis(1)), Duration::ZERO)
    }

    #[test]
    fn a_backed_off_surface_is_not_offered_a_frame_every_tick() {
        let now = Instant::now();
        let (repaint_after, delay) = animating(now);
        assert!(wants_render(false, repaint_after, None, delay, now));
        // Same frame, same zero-delay request, and the only difference is
        // that the surface has shown it will refuse: the unpaced submit loop
        // is exactly this assertion coming back true.
        let armed = now.checked_add(SURFACE_RETRY);
        assert!(!wants_render(false, repaint_after, armed, delay, now));
    }

    #[test]
    fn a_backed_off_surface_is_still_offered_a_frame_each_interval() {
        let now = Instant::now();
        // An idle UI, whose own deadline is an interval away and whose delay
        // is not zero: with the backoff gone this tick paints nothing, so a
        // due retry is the only thing that can explain the frame — which is
        // what makes this measure the poll rather than agree with it.
        let idle = Duration::from_secs(1);
        let ahead = now.checked_add(idle);
        assert!(!wants_render(false, ahead, None, idle, now));
        // Armed one interval ago, so this tick tries the acquire — the offer
        // whose success is the only thing that has to be believed for a
        // window with no `Occluded(false)` to come back.
        assert!(wants_render(false, ahead, now.checked_sub(SURFACE_RETRY), idle, now));
    }

    #[test]
    fn a_transient_refusal_still_retries_on_the_next_tick() {
        let now = Instant::now();
        let (_, delay) = animating(now);
        // A resize's surface goes Outdated for a frame or two. Every refusal
        // below the threshold must leave the backoff unarmed, so the retry is
        // the very next tick and the drag stays smooth — 250 ms of stale
        // content per hitch is exactly what this must not buy.
        let mut refused = 0;
        for _ in 1..REFUSALS_BEFORE_BACKOFF {
            let retry;
            (refused, retry) = after_render(false, refused, now);
            assert_eq!(retry, None);
            assert!(wants_render(false, Some(now), retry, delay, now));
        }
        // One more, and it is no longer transient.
        let (_, retry) = after_render(false, refused, now);
        assert!(retry.is_some());
        assert!(!wants_render(false, Some(now), retry, delay, now));
    }

    #[test]
    fn one_present_ends_the_backoff() {
        let now = Instant::now();
        let (repaint_after, delay) = animating(now);
        // However long it has been refusing — this is the recovery that has
        // to work without any occlusion event arriving to announce it.
        let (refused, retry) = after_render(false, u32::MAX - 1, now);
        assert!(retry.is_some());
        assert!(!wants_render(false, repaint_after, retry, delay, now));
        let (refused, retry) = after_render(true, refused, now);
        assert_eq!((refused, retry), (0, None));
        assert!(wants_render(false, repaint_after, retry, delay, now));
    }

    #[test]
    fn a_texture_delta_is_uploaded_even_while_backed_off() {
        let now = Instant::now();
        let idle = Duration::from_secs(1);
        // Not due, and the UI is asking for nothing, so this tick paints
        // nothing at all — which is what would drop the atlas upload for good.
        let armed = now.checked_add(SURFACE_RETRY);
        let ahead = now.checked_add(idle);
        assert!(!wants_render(false, ahead, armed, idle, now));
        assert!(wants_render(true, ahead, armed, idle, now));
    }

    #[test]
    fn a_presenting_window_keeps_its_deadline() {
        let now = Instant::now();
        let capped = Duration::from_millis(16);
        // A frame-rate cap, with no backoff because the surface is taking
        // frames: the deadline is ahead, so this tick waits.
        assert!(!wants_render(false, now.checked_add(capped), None, capped, now));
        // Reached, so it paints.
        assert!(wants_render(false, now.checked_sub(capped), None, capped, now));
        // No deadline yet and nothing urgent asked for: still waits.
        assert!(!wants_render(false, None, None, capped, now));
    }
}

impl<State, U> WindowHandler for EguiWindow<State, U>
where
    State: 'static + Send,
    U: FnMut(&mut egui::Ui, &mut Queue, &mut State),
    U: 'static + Send,
{
    fn on_frame(&mut self, window: &mut Window) {
        // The whole callback, end to end. Every other reading measures a STAGE
        // of it; this measures the thing the frame timer actually has to fit
        // inside its period, so `tick` against the interval between ticks
        // separates "the work is slow" from "we are not being called".
        let tick_start = Instant::now();
        let Some(state) = &mut self.user_state else {
            return;
        };

        // BEFORE the input the frame is laid out from: a size this window has
        // already been given from outside (see `SizeSource`). Read after it,
        // the frame is built for a window that has stopped being that size,
        // and lands in a surface that is already the new one.
        let old_physical_size = self.physical_size;
        if let Some(size) = self.size_source.as_ref().and_then(SizeSource::take) {
            let adopted = PhySize::new(
                (size.width * self.pixels_per_point as f64).round().max(1.0) as u32,
                (size.height * self.pixels_per_point as f64).round().max(1.0) as u32,
            );
            self.physical_size = adopted;
        }

        self.egui_input.time = Some(self.start_time.elapsed().as_secs_f64());
        let screen_rect = calculate_screen_rect(self.physical_size, self.points_per_pixel);
        self.egui_input.screen_rect = Some(screen_rect);
        if let Some(viewport) = self.egui_input.viewports.get_mut(&self.viewport_id) {
            viewport.inner_rect = Some(screen_rect);
        }

        //let mut repaint_requested = false;
        let mut frame_interval = None;
        let mut queue = Queue::new(
            &mut self.bg_color,
            &mut self.close_requested,
            &mut self.physical_size,
            &mut self.key_capture,
            &mut frame_interval,
            window.display_max_fps(),
            self.tess_ms,
            self.draw_gpu_ms,
            self.acquire_ms,
            self.tick_ms,
            self.render_ms,
            self.upload_ms,
            self.ubuf_ms,
            self.texture_ms,
            self.encode_ms,
            self.submit_ms,
            self.prims,
            self.verts,
        );

        let mut full_output = self.egui_ctx.run_ui(self.egui_input.take(), |ui| {
            (self.user_update)(ui, &mut queue, state)
        });

        // Re-arming replaces the timer that is currently firing this very
        // callback. Its `Drop` only unregisters it from the run loop, which is
        // documented as safe from within a timer callback, and the closure is
        // owned by the timer rather than borrowed from here.
        if let Some(interval) = frame_interval {
            window.set_frame_interval(interval);
        }

        if self.close_requested {
            window.close();
        }

        // Prevent data from being allocated every frame by storing this
        // in a member field.

        let Some(viewport_output) = full_output.viewport_output.get(&self.viewport_id) else {
            // The main window was closed by egui.
            window.close();
            return;
        };

        for command in viewport_output.commands.iter() {
            match command {
                ViewportCommand::Close => {
                    window.close();
                }
                ViewportCommand::InnerSize(size) => window.resize(baseview::Size {
                    width: size.x.max(1.0) as f64,
                    height: size.y.max(1.0) as f64,
                }),
                _ => {}
            }
        }

        if self.physical_size != old_physical_size {
            // As in `new()`: convert physical pixels to the logical points
            // `Window::resize()` expects.
            window.resize(baseview::Size {
                width: (self.physical_size.width.max(1) as f64) * self.points_per_pixel as f64,
                height: (self.physical_size.height.max(1) as f64) * self.points_per_pixel as f64,
            });
        }

        let now = Instant::now();
        let has_texture_updates = !full_output.textures_delta.set.is_empty()
            || !full_output.textures_delta.free.is_empty();
        // Copied out of the borrow so it stays readable after `render()` takes
        // `&mut full_output` below (Duration is Copy, so this costs nothing).
        let repaint_delay = viewport_output.repaint_delay;
        let do_repaint_now = wants_render(
            has_texture_updates,
            self.repaint_after,
            self.retry_after,
            repaint_delay,
            now,
        );

        if do_repaint_now {
            // The renderer half of the callback, whole. `tick` minus this is
            // the egui half — the UI closure plus egui's own end-of-pass work
            // — so between them nothing in the frame is unattributed, even
            // though neither is a single stage.
            let render_start = Instant::now();
            let presented = self.renderer.render(
                window,
                self.bg_color,
                self.physical_size,
                self.pixels_per_point,
                &mut self.egui_ctx,
                &mut full_output,
            );

            self.render_ms = render_start.elapsed().as_secs_f32() * 1000.0;

            // A skipped present (occluded window, lost/outdated surface)
            // must not consume the repaint request: retry next tick, so
            // the first frame after the surface comes back is fresh
            // rather than the pre-occlusion ghost.
            //
            // On a successful paint, schedule the next deadline from THIS
            // instant rather than leaving it unset. Clearing it to `None`
            // costs a whole tick: the deadline would only be established on
            // the following tick, from that later `now`, so every capped
            // interval silently ran one tick long.
            self.repaint_after =
                if presented { now.checked_add(repaint_delay) } else { Some(now) };

            // A present PROVES the surface is available — the acquire refuses
            // an occluded window — so it is what the backoff defers to, and
            // the only thing that has to be believed for the window to come
            // back. See `after_render`.
            (self.refused_presents, self.retry_after) =
                after_render(presented, self.refused_presents, now);
            self.tess_ms = self.renderer.last_tess_ms();
            self.draw_gpu_ms = self.renderer.last_gpu_ms();
            self.acquire_ms = self.renderer.last_acquire_ms();
            self.upload_ms = self.renderer.last_upload_ms();
            self.ubuf_ms = self.renderer.last_ubuf_ms();
            self.texture_ms = self.renderer.last_texture_ms();
            self.prims = self.renderer.last_prims();
            self.verts = self.renderer.last_verts();
            self.encode_ms = self.renderer.last_encode_ms();
            self.submit_ms = self.renderer.last_submit_ms();
        } else if let Some(candidate) = now.checked_add(repaint_delay) {
            // Keep the EARLIEST pending deadline rather than overwriting it.
            //
            // egui recomputes `repaint_delay` from scratch on every pass (it
            // resets to MAX in `begin_pass_repaint_logic` and takes the min of
            // that pass's requests), and the UI closure runs on every tick —
            // including ticks that paint nothing. Overwriting meant a steady
            // `request_repaint_after(N)` re-based the deadline to `now + N` on
            // each tick, so for any N longer than the tick interval `now`
            // never caught up and the deadline receded forever: the window
            // stopped painting until an input event or a texture upload forced
            // it. That silently disabled every delayed repaint, from the idle
            // poll to a frame-rate cap.
            self.repaint_after =
                Some(self.repaint_after.map_or(candidate, |pending| pending.min(candidate)));
        }

        for command in full_output.platform_output.commands {
            match command {
                egui::OutputCommand::CopyText(text) => {
                    if let Some(clipboard_ctx) = &mut self.clipboard_ctx
                        && let Err(err) = clipboard_ctx.set_contents(text)
                    {
                        error!("Copy/Cut error: {}", err);
                    }
                }
                egui::OutputCommand::CopyImage(_) => {
                    warn!("Copying images is not supported in egui_baseview.");
                }
                egui::OutputCommand::OpenUrl(open_url) => {
                    if let Err(err) = open::that_detached(&open_url.url) {
                        error!("Open error: {}", err);
                    }
                }
            }
        }

        let cursor_icon =
            crate::translate::translate_cursor_icon(full_output.platform_output.cursor_icon);
        if self.current_cursor_icon != cursor_icon {
            self.current_cursor_icon = cursor_icon;

            window.set_mouse_cursor(cursor_icon);
        }

        // A temporary workaround for keyboard input not working sometimes.
        // See https://github.com/BillyDM/egui-baseview/issues/20
        #[cfg(feature = "keyboard_focus_workaround")]
        {
            if !full_output.platform_output.events.is_empty()
                || full_output.platform_output.ime.is_some()
            {
                window.focus();
            }
        }
        self.tick_ms = tick_start.elapsed().as_secs_f32() * 1000.0;
    }

    #[allow(unused_variables)]
    fn on_event(&mut self, window: &mut Window, event: Event) -> EventStatus {
        let mut return_status = EventStatus::Captured;

        // Parent/embedded windows do not always gain keyboard focus
        // Automatically on click. Request focus explicitly before forwarding the event.
        if matches!(
            event,
            Event::Mouse(baseview::MouseEvent::ButtonPressed { .. })
        ) && !window.has_focus()
        {
            window.focus();
        }

        match &event {
            baseview::Event::Mouse(event) => match event {
                baseview::MouseEvent::CursorMoved {
                    position,
                    modifiers,
                } => {
                    self.update_modifiers(modifiers);

                    let pos = pos2(position.x as f32, position.y as f32);
                    self.pointer_pos_in_points = Some(pos);
                    self.egui_input.events.push(egui::Event::PointerMoved(pos));
                }
                baseview::MouseEvent::ButtonPressed { button, modifiers } => {
                    self.update_modifiers(modifiers);

                    if let Some(pos) = self.pointer_pos_in_points
                        && let Some(button) = crate::translate::translate_mouse_button(*button)
                    {
                        self.egui_input.events.push(egui::Event::PointerButton {
                            pos,
                            button,
                            pressed: true,
                            modifiers: self.egui_input.modifiers,
                        });
                    }
                }
                baseview::MouseEvent::ButtonReleased { button, modifiers } => {
                    self.update_modifiers(modifiers);

                    if let Some(pos) = self.pointer_pos_in_points
                        && let Some(button) = crate::translate::translate_mouse_button(*button)
                    {
                        self.egui_input.events.push(egui::Event::PointerButton {
                            pos,
                            button,
                            pressed: false,
                            modifiers: self.egui_input.modifiers,
                        });
                    }
                }
                baseview::MouseEvent::WheelScrolled {
                    delta: scroll_delta,
                    modifiers,
                } => {
                    self.update_modifiers(modifiers);

                    #[allow(unused_mut)]
                    let (unit, mut delta) = match scroll_delta {
                        baseview::ScrollDelta::Lines { x, y } => {
                            (egui::MouseWheelUnit::Line, egui::vec2(*x, *y))
                        }

                        baseview::ScrollDelta::Pixels { x, y } => (
                            egui::MouseWheelUnit::Point,
                            egui::vec2(*x, *y) * self.points_per_pixel,
                        ),
                    };

                    if cfg!(target_os = "macos") {
                        // This is still buggy in winit despite
                        // https://github.com/rust-windowing/winit/issues/1695 being closed
                        //
                        // TODO: See if this is an issue in baseview as well.
                        delta.x *= -1.0;
                    }

                    self.egui_input.events.push(egui::Event::MouseWheel {
                        unit,
                        delta,
                        modifiers: self.egui_input.modifiers,
                        phase: egui::TouchPhase::Move,
                    });
                }
                baseview::MouseEvent::CursorLeft => {
                    // `PointerGone` is egui's business; the last position is
                    // ours, and forgetting it here is how a release goes
                    // missing. A button event is only ever sent with a
                    // position, so one that arrives after the exit — the
                    // release of a drag let go outside the window — would be
                    // dropped, and egui would believe the button is held
                    // forever: every ScrollArea in the editor refuses the
                    // wheel while anything at all is being dragged.
                    self.egui_input.events.push(egui::Event::PointerGone);
                }
                _ => {}
            },
            baseview::Event::Keyboard(event) => {
                // The set the event carries rather than a toggle per modifier
                // key, as for mouse events: a key-up that went to another
                // window is corrected by the next event of either kind.
                self.update_modifiers(&event.modifiers);

                let pressed = event.state == keyboard_types::KeyState::Down;

                if let Some(key) = crate::translate::translate_virtual_key(&event.key) {
                    self.egui_input.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: event.repeat,
                        modifiers: self.egui_input.modifiers,
                    });
                }

                if pressed {
                    // VirtualKeyCode::Paste etc in winit are broken/untrustworthy,
                    // so we detect these things manually:
                    //
                    // TODO: See if this is an issue in baseview as well.
                    if is_cut_command(self.egui_input.modifiers, event.code) {
                        self.egui_input.events.push(egui::Event::Cut);
                    } else if is_copy_command(self.egui_input.modifiers, event.code) {
                        self.egui_input.events.push(egui::Event::Copy);
                    } else if is_paste_command(self.egui_input.modifiers, event.code) {
                        if let Some(clipboard_ctx) = &mut self.clipboard_ctx {
                            match clipboard_ctx.get_contents() {
                                Ok(contents) => {
                                    self.egui_input.events.push(egui::Event::Text(contents))
                                }
                                Err(err) => {
                                    error!("Paste error: {}", err);
                                }
                            }
                        }
                    } else if let keyboard_types::Key::Character(written) = &event.key
                        && !self.egui_input.modifiers.ctrl
                        && !self.egui_input.modifiers.command
                    {
                        self.egui_input
                            .events
                            .push(egui::Event::Text(written.clone()));
                    }
                }

                match &self.key_capture {
                    KeyCapture::CaptureAll => {}
                    KeyCapture::IgnoreAll => return_status = EventStatus::Ignored,
                    KeyCapture::CaptureKeys(keys) => {
                        if !keys.contains(&event.key) {
                            return_status = EventStatus::Ignored
                        }
                    }
                    KeyCapture::IgnoreKeys(keys) => {
                        if keys.contains(&event.key) {
                            return_status = EventStatus::Ignored
                        }
                    }
                }
            }
            baseview::Event::Window(event) => match event {
                baseview::WindowEvent::Resized(window_info) => {
                    self.pixels_per_point = match self.scale_policy {
                        WindowScalePolicy::ScaleFactor(scale) => scale,
                        WindowScalePolicy::SystemScaleFactor => window_info.scale(),
                    } as f32;
                    self.points_per_pixel = self.pixels_per_point.recip();

                    self.physical_size = window_info.physical_size();

                    let screen_rect =
                        calculate_screen_rect(self.physical_size, self.points_per_pixel);

                    self.egui_input.screen_rect = Some(screen_rect);

                    let viewport_info = self
                        .egui_input
                        .viewports
                        .get_mut(&self.viewport_id)
                        .unwrap();
                    viewport_info.native_pixels_per_point = Some(self.pixels_per_point);
                    viewport_info.inner_rect = Some(screen_rect);

                    // Schedule to repaint on the next frame.
                    self.repaint_after = Some(Instant::now());
                }
                baseview::WindowEvent::Focused => {
                    self.egui_input
                        .events
                        .push(egui::Event::WindowFocused(true));
                    self.egui_input
                        .viewports
                        .get_mut(&self.viewport_id)
                        .unwrap()
                        .focused = Some(true);
                }
                baseview::WindowEvent::Unfocused => {
                    self.egui_input
                        .events
                        .push(egui::Event::WindowFocused(false));
                    self.egui_input
                        .viewports
                        .get_mut(&self.viewport_id)
                        .unwrap()
                        .focused = Some(false);
                }
                baseview::WindowEvent::Occluded(occluded) => {
                    // The renderer's half of it: what the window shows for the
                    // frame between coming back and having something fresh to
                    // show. See the wgpu renderer's `layer_present`.
                    self.renderer.window_occluded(*occluded);
                    if !occluded {
                        // Re-exposed after occlusion: the compositor may
                        // have kept showing a stale snapshot of the
                        // window; repaint and present a fresh frame now.
                        self.repaint_after = Some(Instant::now());
                        // And drop the backoff the occlusion earned, so that
                        // frame is not held behind it. This is a shortcut,
                        // never the way back: the poll recovers a window whose
                        // `Occluded(false)` never arrives, which is why
                        // `wants_render` is keyed on refusals and not on this.
                        self.refused_presents = 0;
                        self.retry_after = None;
                    }
                }
                baseview::WindowEvent::WillClose => {}
            },
        }

        // For keyboard events, also check if egui actually wants keyboard input
        // This allows DAW shortcuts (spacebar, etc.) to pass through when no text field is focused
        match &event {
            baseview::Event::Keyboard(_) => {
                if return_status == EventStatus::Captured
                    && !self.egui_ctx.egui_wants_keyboard_input()
                {
                    EventStatus::Ignored
                } else {
                    return_status
                }
            }
            baseview::Event::Mouse(_) => {
                if self.egui_ctx.egui_is_using_pointer() || self.egui_ctx.egui_wants_pointer_input()
                {
                    EventStatus::Captured
                } else {
                    EventStatus::Ignored
                }
            }
            baseview::Event::Window(_) => EventStatus::Captured,
        }
    }
}

fn is_cut_command(modifiers: egui::Modifiers, keycode: keyboard_types::Code) -> bool {
    (modifiers.command && keycode == keyboard_types::Code::KeyX)
        || (cfg!(target_os = "windows")
            && modifiers.shift
            && keycode == keyboard_types::Code::Delete)
}

fn is_copy_command(modifiers: egui::Modifiers, keycode: keyboard_types::Code) -> bool {
    (modifiers.command && keycode == keyboard_types::Code::KeyC)
        || (cfg!(target_os = "windows")
            && modifiers.ctrl
            && keycode == keyboard_types::Code::Insert)
}

fn is_paste_command(modifiers: egui::Modifiers, keycode: keyboard_types::Code) -> bool {
    (modifiers.command && keycode == keyboard_types::Code::KeyV)
        || (cfg!(target_os = "windows")
            && modifiers.shift
            && keycode == keyboard_types::Code::Insert)
}

/// Calculate screen rectangle in logical size.
fn calculate_screen_rect(physical_size: PhySize, points_per_pixel: f32) -> Rect {
    let logical_size = (
        physical_size.width as f32 * points_per_pixel,
        physical_size.height as f32 * points_per_pixel,
    );
    Rect::from_min_size(Pos2::new(0f32, 0f32), vec2(logical_size.0, logical_size.1))
}
