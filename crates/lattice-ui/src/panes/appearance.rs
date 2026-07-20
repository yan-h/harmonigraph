//! The Appearance pane: how a sounding note is drawn — core, octaves,
//! melody/bass marks, home grid, color, labels, effects.

use super::{param_bar, section};
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{button_row, choice_row, ValueBar};
use crate::SharedState;
use lattice_scene::{
    HighlightExtremes, IdleMarker, MarkContrast, MarkPlace, MarkRecede, MarkStyle, NodeStyle,
    OuterStyle,
};

/// Cosmetic settings, apart from the structural View pane: how a sounding
/// note is drawn, colored, and faded — not what the grid shows. Laid out
/// top to bottom as the note itself reads outward, then its color/timing,
/// then overlays: Core (the mark at the node's center) and Octaves (the
/// ring of octave indicators around it) are the two independent rendering
/// layers; Color and Fade set how notes are tinted and how they linger;
/// Labels is the note text; Effects are scene-wide extras. Scrolls so the
/// full list is reachable in a short pane.
pub(super) fn appearance_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Core: the mark at a sounding node's center. One continuous
            // shape sized by the radius (0 = off, like Bloom) and morphed by
            // Solidity from a soft glow (0) to the classic solid orb (1),
            // painted per the Style row. Independent of the Octaves layer.
            ui.heading("Core");
            ValueBar::new(&mut state.view.core_radius, 0.0..=0.9, "Radius")
                .show(ui)
                .on_hover_text(
                    "Core size (disc and glow together); 0 turns it off, \
                     0.46 is the classic disc",
                );
            ui.add_enabled_ui(state.view.core_radius > 0.0, |ui| {
                ValueBar::new(&mut state.view.core_solidity, 0.0..=1.0, "Solidity")
                    .show(ui)
                    .on_hover_text(
                        "0 = a soft glow, 1 = the classic solid orb; in \
                         between the disc fades in over its glow and its \
                         edge crisps",
                    );
                // Switchable paints (idle nodes look the same in all).
                // Steady is a calm solid disc blending the sounding octaves'
                // colors; the rest are field styles — Vortex the gas look,
                // Checker/Spiral/Pinwheel patterns on the sphere. The paint
                // dissolves with the disc toward the glow end.
                choice_row(
                    ui,
                    "Style",
                    &mut state.view.node_style,
                    &[
                        (NodeStyle::Steady, "Steady", ""),
                        (NodeStyle::Vortex, "Vortex", ""),
                        (NodeStyle::Checker, "Checker", ""),
                        (NodeStyle::Spiral, "Spiral", ""),
                        (NodeStyle::Pinwheel, "Pinwheel", ""),
                    ],
                );
            });

            // Octaves: which octaves of the pitch class are sounding, shown
            // as glyphs at each note's absolute-pitch angle within a radial
            // band. Independent of the Core.
            section(ui, "Octaves");
            // One glyph shape (ring sectors) — the alternatives were
            // switchable for live comparison and have been settled — so this
            // is just whether the layer draws. The bool goes through the
            // OuterStyle enum the persist and the shader still speak.
            let mut show = state.view.outer_style != OuterStyle::Off;
            if ui
                .checkbox(&mut show, "Show octaves")
                .on_hover_text(
                    "Ring sectors spanning the band, one per sounding octave, \
                     each at its own pitch's angle; band inner 0 = pie wedges",
                )
                .changed()
            {
                state.view.outer_style = if show { OuterStyle::Slices } else { OuterStyle::Off };
            }
            // If the band bars cross, the scene keeps outer ahead of inner
            // rather than collapsing.
            ui.add_enabled_ui(show, |ui| {
                ValueBar::new(&mut state.view.outer_inner, 0.0..=0.9, "Band inner")
                    .show(ui)
                    .on_hover_text("Octave band's inner radius; 0 reaches the center");
                ValueBar::new(&mut state.view.outer_outer, 0.2..=1.0, "Band outer")
                    .show(ui)
                    .on_hover_text("Octave band's outer radius");
                ValueBar::new(&mut state.view.outer_solidity, 0.0..=1.0, "Solidity")
                    .show(ui)
                    .on_hover_text(
                        "0 = soft glowy octave marks, 1 = the crisp classic \
                         shapes; only softens the glyph edges, shapes and \
                         angles stay put",
                    );
                // One padding for the whole layer: between sectors, and
                // between the band and the melody/bass rings.
                ValueBar::new(&mut state.view.outer_gap, 0.0..=0.4, "Gap")
                    .show(ui)
                    .on_hover_text(
                        "Padding inside the octave layer: between one octave \
                         and the next, and between the band and the \
                         melody/bass rings. 0 closes the octaves into a solid \
                         annulus and seats the rings against it. Wide values \
                         push the bass ring in toward the core -- raise Band \
                         inner to make room",
                    );
                // Backdrop: draw the silent octaves faintly so a lone octave
                // still reads as a whole note.
                ValueBar::new(&mut state.view.outer_backdrop, 0.0..=1.0, "Backdrop")
                    .show(ui)
                    .on_hover_text(
                        "Complete the octave ring: draw the silent octaves \
                         faintly behind the sounding sectors, so a lone octave \
                         still reads as a whole note. 0 = off",
                    );
            });

            // Melody / bass: mark the outer held notes so a chord's top and
            // bottom line read at a glance.
            section(ui, "Melody / bass");
            choice_row(
                ui,
                "Mark",
                &mut state.view.highlight_extremes,
                &[
                    (HighlightExtremes::Off, "Off", "No melody or bass mark"),
                    (HighlightExtremes::Melody, "Melody", "Mark the highest held note"),
                    (HighlightExtremes::Bass, "Bass", "Mark the lowest held note"),
                    (
                        HighlightExtremes::Both,
                        "Both",
                        "Mark both. Each ring takes its own note's color and \
                         they are told apart by radius, so a note that is at \
                         once the highest and the lowest -- a lone held note, \
                         or a chord whose top and bottom share a pitch class -- \
                         simply gets both",
                    ),
                ],
            );
            // The mark is a stripe down one angular SIDE of the marked
            // note's own sector: the melody along the edge facing the next
            // octave up, the bass along the one facing the next down --
            // which rhymes with the pitch mapping the lattice already has.
            // A lone note is its own melody AND bass, so it takes both
            // sides and keeps its own color between them.
            ui.add_enabled_ui(state.view.highlight_extremes != HighlightExtremes::Off, |ui| {
                ValueBar::new(&mut state.view.mark_width, 0.0..=0.45, "Width")
                    .show(ui)
                    .on_hover_text(
                        "How much of the mark there is: the stripe's width as a \
                         fraction of its sector, how far Widen spreads, or how \
                         far Emphasis carries the inner voices down. 0 leaves \
                         no mark at all",
                    );
                // The ring already encodes pitch as angle, so the mark does
                // not have to say WHICH of the two it is -- only which two
                // they are. Emphasis and Widen spend nothing to do that.
                choice_row(
                    ui,
                    "Style",
                    &mut state.view.mark_style,
                    &[
                        (
                            MarkStyle::Stripe,
                            "Stripe",
                            "A white stripe down one side of the marked sector",
                        ),
                        (
                            MarkStyle::Emphasis,
                            "Emphasis",
                            "Nothing added: the INNER voices dim instead, leaving \
                             the outer two at full. Cannot wash out on any note, \
                             and costs nothing with one or two notes held -- \
                             every note is an extreme then, so nothing dims",
                        ),
                        (
                            MarkStyle::Widen,
                            "Widen",
                            "The marked sector spans a wider angle, growing \
                             toward its own side -- into the gap it already \
                             has, so nothing is reserved for it",
                        ),
                        (
                            MarkStyle::Glow,
                            "Glow",
                            "Lift the marked sector until it crosses the bloom \
                             threshold, so it alone halos. Emphasis by light \
                             rather than color or shape; needs Bloom above 0 \
                             for the halo",
                        ),
                        (
                            MarkStyle::Pulse,
                            "Pulse",
                            "The marked sector breathes, slowly, on the same \
                             clock the field styles use. Motion is the one \
                             channel nothing else on the node is using",
                        ),
                        (
                            MarkStyle::Sweep,
                            "Sweep",
                            "A bright band runs along the marked sector -- \
                             outward for the melody, inward for the bass, so \
                             the motion says which end it is. It lifts toward \
                             white, so it reads on dark notes and barely on \
                             pale ones",
                        ),
                        (
                            MarkStyle::Throb,
                            "Throb",
                            "The marked sector breathes in SIZE rather than \
                             brightness, widening and narrowing on the beat",
                        ),
                        (
                            MarkStyle::Focus,
                            "Focus",
                            "The marked sector stays crisp while the rest \
                             soften -- the outer voices in focus, the inner \
                             ones behind them. No ink, no space, no color",
                        ),
                    ],
                );
                // Emphasis only: how the inner voices give way. All three
                // act on color, never on coverage -- see MarkRecede.
                // Pulse, Sweep and Throb share one clock.
                ui.add_enabled_ui(
                    matches!(
                        state.view.mark_style,
                        MarkStyle::Pulse | MarkStyle::Sweep | MarkStyle::Throb
                    ),
                    |ui| {
                        ValueBar::new(&mut state.view.mark_rate, 0.1..=4.0, "Rate")
                            .show(ui)
                            .on_hover_text(
                                "Cycles per second for the animated marks. They \
                                 run on global time, so a retrigger never \
                                 restarts the motion and every marked note \
                                 moves together",
                            );
                    },
                );
                ui.add_enabled_ui(state.view.mark_style == MarkStyle::Emphasis, |ui| {
                    choice_row(
                        ui,
                        "Recede",
                        &mut state.view.mark_recede,
                        &[
                            (
                                MarkRecede::Grey,
                                "Grey",
                                "Drain the color, keep the brightness: the inner \
                                 voices stay just as visible and give up only \
                                 their hue, so the outer two are the only \
                                 sectors still in color",
                            ),
                            (
                                MarkRecede::Dim,
                                "Dim",
                                "Darken, keep the hue: every voice keeps its \
                                 pitch color, the inner ones just sit back",
                            ),
                            (MarkRecede::Both, "Both", "Drained and darkened"),
                        ],
                    );
                });
                ui.add_enabled_ui(state.view.mark_style == MarkStyle::Stripe, |ui| {
                choice_row(
                    ui,
                    "Place",
                    &mut state.view.mark_place,
                    &[
                        (
                            MarkPlace::Inside,
                            "Inside",
                            "Carved out of the marked note's own sector, along \
                             its edge",
                        ),
                        (
                            MarkPlace::Outside,
                            "Outside",
                            "Laid alongside the sector instead, just past its \
                             edge, leaving the note's wedge whole. Drawn under \
                             the octaves, so a lit neighbour still wins",
                        ),
                    ],
                );
                // White is the stripe's color; what is open is how it ENDS.
                // White alone dies against the pale top of the pitch ramp,
                // which is exactly where the melody mark lands, so the
                // contrast comes from a dark boundary instead of the fill.
                choice_row(
                    ui,
                    "Contrast",
                    &mut state.view.mark_contrast,
                    &[
                        (
                            MarkContrast::Gap,
                            "Gap",
                            "A gap where the white meets the note -- the same \
                             device as the gaps between octaves, thinner. \
                             Nothing is painted, so nothing can be the wrong \
                             color against the note",
                        ),
                        (
                            MarkContrast::Gradient,
                            "Gradient",
                            "The white ramps to dark across the stripe, ending on \
                             the same boundary with no seam",
                        ),
                        (
                            MarkContrast::Off,
                            "None",
                            "Plain white -- legible on every note but the palest",
                        ),
                    ],
                );
                ui.add_enabled_ui(state.view.mark_contrast != MarkContrast::Off, |ui| {
                    ValueBar::new(&mut state.view.mark_keyline, 0.0..=0.2, "Keyline")
                        .show(ui)
                        .on_hover_text(
                            "How wide that gap is, in the same units as the \
                             octaves' Gap and the band radii. Constant at every \
                             radius, and centred on the line where the stripe \
                             meets the note -- the one that runs to the slice's \
                             point",
                        );
                });
                });
            });

            // Home grid: the always-drawn structural layer -- the faint
            // lines between node positions AND the idle marker sitting at
            // each unlit home-sheet node. Idle positions draw no disc, so
            // together these are what carry the lattice's shape when
            // nothing is playing. They share one color for that reason.
            section(ui, "Home grid");
            button_row(ui, |ui| {
                ui.label("Color");
                ui.color_edit_button_rgba_unmultiplied(&mut state.view.grid_color)
                    .on_hover_text(
                        "Color of the whole idle structure -- grid lines and \
                         idle node markers alike. The alpha is how faint an \
                         unlit LINE draws; markers keep their own presence. \
                         Lit segments still take their notes' color",
                    );
            });
            ValueBar::new(&mut state.view.grid_thickness, 0.0..=4.0, "Thickness")
                .show(ui)
                .on_hover_text("Line width, as a multiple of the classic hairline");
            ValueBar::new(&mut state.view.grid_inset, 0.0..=3.0, "Line gap")
                .show(ui)
                .on_hover_text(
                    "How far each line stops short of the node it runs to, as \
                     a multiple of the node radius; 0 runs it to the center",
                );
            ui.checkbox(&mut state.view.grid_dashed, "Dashed").on_hover_text(
                "Dash the in-plane lines. The sevens-axis links are always \
                 dashed -- that's what marks them as depth links",
            );

            // The idle marker: shown ALWAYS at each unlit home-sheet node,
            // independent of the active appearance and of whether a note
            // plays there (a sounding note just draws over it). Off-sheet
            // positions are marked by the lines alone.
            choice_row(
                ui,
                "Marker",
                &mut state.view.idle_marker,
                &[
                    (IdleMarker::None, "None", "No idle marker"),
                    (IdleMarker::Dot, "Dot", "A filled dot at the radius below"),
                    (
                        IdleMarker::Circle,
                        "Circle",
                        "A thin outline circle at the radius below",
                    ),
                ],
            );
            ui.add_enabled_ui(state.view.idle_marker != IdleMarker::None, |ui| {
                ValueBar::new(&mut state.view.idle_radius, 0.0..=0.9, "Marker radius")
                    .show(ui)
                    .on_hover_text(
                        "Size of the idle marker; independent of the active \
                         Core (0.46 is the classic placeholder ring)",
                    );
            });

            // Color: the pitch->color gradient endpoints (MIDI notes) the
            // pitch-colored channels map through.
            section(ui, "Color");
            for &key in &ParamKey::COLOR {
                param_bar(ui, params, key);
            }

            // Fade: how long a released note lingers. One time for the whole
            // node — core, octave glyphs, and melody/bass marks — rather than
            // one per layer, so a release reads as a single gesture instead
            // of pieces of the node going dark at different moments.
            section(ui, "Fade");
            param_bar(ui, params, ParamKey::Fade).on_hover_text(
                "Seconds a released note keeps fading — the pitch class core, \
                 the octave glyphs, and the melody/bass marks together. 0 cuts \
                 notes off the moment they're released",
            );

            // Labels: the note text drawn on hovered and sounding nodes.
            section(ui, "Labels");
            ui.checkbox(&mut state.view.show_labels, "Note names");
            // Cents ride on the labels, so the toggle grays out with them off.
            ui.add_enabled(
                state.view.show_labels,
                egui::Checkbox::new(&mut state.view.show_cents, "Cents"),
            );

            // Effects: scene-wide extras layered over the notes.
            section(ui, "Effects");
            // 0 = off (the renderer skips the whole post-process chain), so
            // the bar doubles as the toggle.
            ValueBar::new(&mut state.view.bloom_strength, 0.0..=1.5, "Bloom")
                .show(ui)
                .on_hover_text("Soft halo around bright notes; 0 turns the post-process off");
        });
}
