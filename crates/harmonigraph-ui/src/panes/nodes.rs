//! The Nodes pane: how a sounding note is drawn — its core mark, the octave
//! ring around it, the melody/bass marks on the outer held notes, and the
//! color, fade, halo and cleared gutter the whole node wears. Everything here
//! is the *played note*; the surrounding structure and overlays live in
//! [`super::scene`].

use super::{param_bar, section};
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{choice_row, RangeBar, ValueBar};
use crate::SharedState;
use harmonigraph_scene::{
    NodeStyle, OctaveTaper, ViewConfig, MAX_OCTAVE_SPAN, MAX_TAPER_AMOUNT, MIN_OCTAVE_SPAN,
};

/// The sounding-note controls, top to bottom as the note reads outward: the
/// Core mark at its center, the Octaves ring around it, the melody/bass marks
/// on the outer notes, and then the settings that are not about any one of
/// those layers but about all of them at once. Scrolls so the full list is
/// reachable in a short pane.
pub(super) fn nodes_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            core_section(ui, &mut state.view);
            octaves_section(ui, &mut state.view);
            melody_bass_section(ui, &mut state.view);
            every_layer_section(ui, &mut state.view, params);
        });
}

/// Core: the mark at a sounding node's center. One continuous shape sized by
/// the radius (0 = off, like Bloom) and morphed by Solidity from a soft glow
/// (0) to the classic solid orb (1), painted per the Style row. Independent
/// of the Octaves layer.
fn core_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    ui.heading("Core");
    ValueBar::new(&mut view.core_radius, 0.0..=0.9, "Radius")
        .show(ui)
        .on_hover_text(
            "Core size (disc and glow together); 0 turns it off, \
             0.46 is the classic disc",
        );
    ui.add_enabled_ui(view.core_radius > 0.0, |ui| {
        ValueBar::new(&mut view.core_solidity, 0.0..=1.0, "Solidity")
            .show(ui)
            .on_hover_text(
                "0 = a soft glow, 1 = the classic solid orb; in \
                 between the disc fades in over its glow and its \
                 edge crisps",
            );
        // Switchable paints (idle nodes look the same in all).
        // Steady is a calm solid disc blending the sounding octaves'
        // colors; the rest are field styles — Vortex the gas look,
        // Checker and Spiral patterns on the sphere. The paint
        // dissolves with the disc toward the glow end.
        choice_row(
            ui,
            "Style",
            &mut view.node_style,
            &[
                (NodeStyle::Steady, "Steady", ""),
                (NodeStyle::Vortex, "Vortex", ""),
                (NodeStyle::Checker, "Checker", ""),
                (NodeStyle::Spiral, "Spiral", ""),
            ],
        );
    });
}

/// Octaves: which octaves of the pitch class are sounding, shown as arcs of a
/// pitch axis that runs once round the node. Independent of the Core.
fn octaves_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Octaves");
    // Range and Taper are the axis; Band and Gap below are where it is drawn.
    // Range counts octaves out from middle C's and every node draws all of
    // them — the same octave numbers whatever the node's pitch class, which
    // is only what decides where round the turn they land.
    //
    // The buttons ARE the supported range, written out one octave list at a
    // time because each names its own; nothing derives them from the layout's
    // bounds. So widening those bounds without adding a row would leave the
    // extra Range unreachable, and narrowing them would leave a button that
    // sets a span the layout clamps away — a control disagreeing with the
    // wheel it draws. Fail the build on either instead.
    const _: () = assert!(MIN_OCTAVE_SPAN == 2 && MAX_OCTAVE_SPAN == 5);
    choice_row(
        ui,
        "Range",
        &mut view.octave_span,
        &[
            (2, "±2", "C1..C5 around middle C — five octaves to the turn"),
            (3, "±3", "C0..C6"),
            (4, "±4", "C-1..C7"),
            (5, "±5", "C-2..C8 — every octave MIDI has, so one is 33 degrees"),
        ],
    );
    // Uniform is the plain circular division and ignores the amount, so the
    // formulas read as three departures from one baseline rather than four
    // unrelated shapes.
    choice_row(
        ui,
        "Taper",
        &mut view.octave_taper,
        &[
            (
                OctaveTaper::Uniform,
                "Even",
                "An even pitch axis: equal intervals subtend equal angles \
                 anywhere in the window. The amount below is inert",
            ),
            (OctaveTaper::Linear, "Linear", "A straight ramp out from middle C"),
            (
                OctaveTaper::Geometric,
                "Ratio",
                "Each octave out is the same FRACTION of the one inside it: \
                 falls away fastest near the middle, flattening at the edges",
            ),
            (
                OctaveTaper::Plateau,
                "Plateau",
                "Barely narrows the octaves either side of middle C and takes \
                 almost all of the loss at the extremes: a plateau of \
                 full-size middle octaves rather than a gradient",
            ),
        ],
    );
    ui.add_enabled_ui(view.octave_taper != OctaveTaper::Uniform, |ui| {
        ValueBar::new(&mut view.octave_taper_amount, 0.0..=MAX_TAPER_AMOUNT, "Amount")
            .show(ui)
            .on_hover_text(
                "How much of its width the octave at the EDGE of the window \
                 gives up, which the middle ones take: 0 is an even axis, 0.9 \
                 leaves the edge a tenth of the middle. Every formula is \
                 expressed in the same amount, so switching between them \
                 compares their shapes",
            );
    });
    // No on/off: the layer is what says which octaves are sounding, which
    // is the node's whole outer half, and Band already reaches every size
    // from a hairline to the quad edge.
    //
    // Inner and outer radius are one control: the band is the ring between
    // them. Drag either edge, or drag between to slide the ring at a fixed
    // width; the min span keeps it from collapsing.
    ui.label("Band");
    RangeBar::new(&mut view.outer_inner, &mut view.outer_outer, 0.0..=1.0)
        .min_span(0.05)
        .show(ui)
        .on_hover_text(
            "The octave band's inner and outer radius. Inner 0 reaches the \
             node center (pie wedges); drag between the handles to move the \
             whole ring in or out.",
        );
    // One padding for the whole layer: between sectors, and
    // between the band and the melody/bass rings.
    ValueBar::new(&mut view.outer_gap, 0.0..=0.4, "Gap")
        .show(ui)
        .on_hover_text(
            "Padding inside the octave layer: between one octave \
             and the next, and between the band and the \
             melody/bass rings. 0 closes the octaves into a solid \
             annulus and seats the rings against it. Wide values \
             push the bass ring in toward the core -- raise Band \
             inner to make room",
        );
    // No Solidity and no Backdrop bar: both are fixed at 1. The glyphs are
    // always the crisp classic shapes, and the silent octaves always ghost
    // in behind the sounding ones — that backdrop is what completes the
    // ring, so a lone octave still reads as a whole note.
}

/// Melody / bass: mark the outer held notes so a chord's top and bottom line
/// read at a glance.
fn melody_bass_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Melody / bass");
    // Two boxes, not a four-way row: the marks are independent, they are
    // told apart by radius rather than by hue, and a note that is at once
    // the highest and the lowest -- a lone held note, or a chord whose top
    // and bottom share a pitch class -- simply gets both.
    ui.checkbox(&mut view.mark_melody, "Melody")
        .on_hover_text("Ring the highest held note, just inside the octave band");
    ui.checkbox(&mut view.mark_bass, "Bass")
        .on_hover_text("Ring the lowest held note, just outside the octave band");
    // The marks are full rings bracketing the octave band (melody
    // inside, bass outside), each slit either side of the octave
    // responsible so that stretch reads as its own piece.
    ui.add_enabled_ui(view.mark_melody || view.mark_bass, |ui| {
        ValueBar::new(&mut view.mark_thickness, 0.0..=0.3, "Thickness")
            .show(ui)
            .on_hover_text(
                "How thick both mark rings are, in the same units as \
                 the band radii and Gap. 0 turns the rings off; thick \
                 values grow the bass ring in over the core, so raise \
                 Band inner to make room",
            );
    });
}

/// A MIDI note as a key name and octave, for the color-range readout — "C1",
/// "C8" — so the ends read as pitches rather than bare numbers.
fn pitch_readout(midi: f32) -> String {
    let n = midi.round() as i32;
    let name = super::KEY_NAMES[n.rem_euclid(12) as usize];
    format!("{name}{}", harmonigraph_core::notes::display_octave_of(n))
}

/// What every layer of the node shares: the pitch->color gradient it is
/// tinted through, the time it takes to fade on release, the halo it carries
/// while lit, and the gap it clears around the whole of itself.
///
/// One section rather than a heading apiece, because they are one idea — none
/// of them is about the core, the octave glyphs or the melody/bass rings in
/// particular, and all of them apply to whichever of those happen to be drawn.
/// Fade especially: one time for the node rather than one per layer, so a
/// release reads as a single gesture instead of pieces of the node going dark
/// at different moments.
fn every_layer_section(
    ui: &mut egui::Ui,
    view: &mut ViewConfig,
    params: &dyn ParamBackend,
) {
    section(ui, "Every layer");
    ui.label("Color range");
    super::param_range_bar(
        ui,
        params,
        ParamKey::DarkestPitch,
        ParamKey::BrightestPitch,
        0.0..=120.0,
        crate::COLOR_RANGE_MIN_SPAN,
        pitch_readout,
    )
    .on_hover_text(
        "The pitch span the color gradient covers: the low end takes the \
         darkest color, the high end the brightest. Drag either end, or drag \
         between them to slide the whole range.",
    );
    param_bar(ui, params, ParamKey::Fade).on_hover_text(
        "Seconds a released note keeps fading — the pitch class core, \
         the octave glyphs, and the melody/bass marks together. 0 cuts \
         notes off the moment they're released",
    );
    // 0 = off (the renderer skips the whole post-process chain), so the bar
    // doubles as the toggle.
    ValueBar::new(&mut view.bloom_strength, 0.0..=1.5, "Bloom")
        .show(ui)
        .on_hover_text(
            "Soft halo around bright notes; 0 turns the post-process off",
        );
    // Here rather than with the sevens-layer controls, where a "Sevenths" name
    // misreads what it does: the gutter is cleared by every sounding node on
    // every sheet, the home one included and at any sevenths extent, so it
    // belongs to the node and not to the depth axis. The `sevens_` field names
    // stay — they are what saved projects spell.
    ValueBar::new(&mut view.sevens_gutter, 0.0..=0.5, "Gutter")
        .show(ui)
        .on_hover_text(
            "The dark gap a sounding node clears around itself, so it reads \
             over whatever it crosses rather than needing room of its own: \
             the grid lines under it, and the sheets behind it once the \
             lattice has depth. Measured past the node's own edge, and the \
             same width on screen whatever size the node draws at. 0 draws \
             none",
        );
    ValueBar::new(&mut view.sevens_gutter_soft, 0.0..=0.5, "Gutter fade")
        .show(ui)
        .on_hover_text(
            "How gradually the gap ends, independent of how wide it is. \
             0 is a hard edge; past the gutter's own width it softens \
             outward rather than eating into the node",
        );
}
