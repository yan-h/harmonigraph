//! The Appearance pane: how a sounding note is drawn — core, octaves,
//! melody/bass marks, home grid, color, labels, trail, effects.

use super::{param_bar, section};
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{button_row, choice_row, ValueBar};
use crate::SharedState;
use lattice_scene::{HighlightExtremes, IdleMarker, NodeStyle, OuterStyle, TrailMode};

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
            // The marks are full rings bracketing the octave band (bass
            // inside, melody outside), each slit either side of the octave
            // responsible so that stretch reads as its own piece. This
            // fades everything BUT that stretch, from a whole ring down to
            // just the arc over the marked octave.
            ui.add_enabled_ui(state.view.highlight_extremes != HighlightExtremes::Off, |ui| {
                ValueBar::new(&mut state.view.mark_thickness, 0.0..=0.3, "Thickness")
                    .show(ui)
                    .on_hover_text(
                        "How thick both mark rings are, in the same units as \
                         the band radii and Gap. 0 turns the rings off; thick \
                         values grow the bass ring in over the core, so raise \
                         Band inner to make room",
                    );
                ValueBar::new(&mut state.view.mark_unlinked, 0.0..=1.0, "Unlinked")
                    .show(ui)
                    .on_hover_text(
                        "Opacity of the rest of the ring -- the part cut off \
                         from the marked octave's sector. The stretch beside \
                         that sector always draws full. 1 keeps the whole \
                         circle, 0 leaves only the arc over the marked octave. \
                         The slits come from Gap, so Gap 0 leaves nothing to \
                         separate and this does nothing",
                    );
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

            // Trail: the one part of this pane about the past rather than
            // the present. Marks say WHERE the music has been; the path
            // says in what ORDER it went there, and the two are independent
            // on purpose -- either reading is useful without the other.
            section(ui, "Trail");
            choice_row(
                ui,
                "Marks",
                &mut state.view.trail_mode,
                &[
                    (TrailMode::Off, "Off", "Show only what is sounding"),
                    (
                        TrailMode::Ghost,
                        "Ghost",
                        "Every note played stays faintly lit, forever and \
                         evenly -- the lit nodes are the piece's harmonic \
                         territory",
                    ),
                    (
                        TrailMode::Fade,
                        "Fade",
                        "Brightest where the music just was, forgotten over \
                         the Memory time below -- a comet tail rather than a \
                         map",
                    ),
                    (
                        TrailMode::Heat,
                        "Heat",
                        "Brightest where the music has spent the most time: \
                         what a tonal center looks like",
                    ),
                ],
            );
            let marks_on = state.view.trail_mode != TrailMode::Off;
            ui.add_enabled_ui(marks_on, |ui| {
                ui.checkbox(&mut state.view.trail_octaves, "Octaves").on_hover_text(
                    "Remember which octave a note was played in, not just \
                     its pitch class. Also what keeps octaves of one note \
                     apart, since they all share a node",
                );
                ui.checkbox(&mut state.view.trail_labels, "Labels").on_hover_text(
                    "Keep the note name and cents on a visited node, so the \
                     territory can be read off by name. Needs Note names on",
                );
            });

            // The route. Deliberately NOT gated on the marks above.
            ui.checkbox(&mut state.view.trail_path, "Path").on_hover_text(
                "Draw a line from each played note to the next, in playing \
                 order -- the route through the lattice rather than the \
                 places it reached",
            );
            ui.add_enabled_ui(state.view.trail_path, |ui| {
                let mut steps = state.view.trail_path_steps as f32;
                if ValueBar::new(&mut steps, 2.0..=256.0, "Path length")
                    .integer()
                    .show(ui)
                    .on_hover_text("How many recent notes the route connects")
                    .changed()
                {
                    state.view.trail_path_steps = steps as u32;
                }
                ui.checkbox(&mut state.view.trail_path_glow, "Glowing").on_hover_text(
                    "Draw the route as beams instead of thin lines: loud \
                     across a leap, where thin lines suit a dense passage",
                );
                ui.add_enabled(
                    !state.view.trail_path_glow,
                    egui::Checkbox::new(&mut state.view.trail_path_dashed, "Dashed"),
                )
                .on_hover_text("Dash the route, to tell it from the grid's own lines");
            });

            // Shared by both devices, so they stay enabled whenever either
            // is. Memory time is the exception: only Fade and the path
            // measure age at all.
            let trail_on = marks_on || state.view.trail_path;
            ui.add_enabled_ui(trail_on, |ui| {
                ValueBar::new(&mut state.view.trail_strength, 0.0..=1.0, "Strength")
                    .show(ui)
                    .on_hover_text("How loud the past draws -- marks and path alike");
                ValueBar::new(&mut state.view.trail_tint, 0.0..=1.0, "Tint")
                    .show(ui)
                    .on_hover_text(
                        "How far a memory's color is pulled toward the grid: \
                         0 keeps the note's own hue, 1 makes it a grey ghost \
                         of the structure",
                    );
                ui.add_enabled_ui(
                    state.view.trail_mode.uses_time() || state.view.trail_path,
                    |ui| {
                        ValueBar::new(&mut state.view.trail_time, 1.0..=600.0, "Memory")
                            .show(ui)
                            .on_hover_text(
                                "Seconds before a note is forgotten: how long \
                                 a Fade mark lasts, and how far back the path \
                                 reaches. Ghost and Heat never forget",
                            );
                    },
                );
            });
            button_row(ui, |ui| {
                if ui
                    .button("Clear trail")
                    .on_hover_text("Forget everything played so far; sounding notes stay")
                    .clicked()
                {
                    state.tracker.clear_history();
                }
            });

            // Effects: scene-wide extras layered over the notes.
            section(ui, "Effects");
            // 0 = off (the renderer skips the whole post-process chain), so
            // the bar doubles as the toggle.
            ValueBar::new(&mut state.view.bloom_strength, 0.0..=1.5, "Bloom")
                .show(ui)
                .on_hover_text("Soft halo around bright notes; 0 turns the post-process off");
        });
}
