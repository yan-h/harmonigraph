//! Per-tile candidate lists for the glow gather. Rebuilt from this frame's
//! projected nodes and target size; only the GPU allocation is retained.
//! Large halos live in one shared list so index storage is bounded by the
//! tile count plus 64 indices per node, even when every halo fills the pane.

use crate::GpuGlowNode;

pub(super) const TILE: u32 = 32;
const MAX_LOCAL_TILES: u32 = 64;

/// Packed u32s: [columns, global_start, global_end], then tile_count + 1
/// absolute offsets, local indices, and global indices. Each list preserves
/// node order; the shader merges the two sorted lists to keep rounding stable.
pub(super) fn pack(nodes: &[GpuGlowNode], size: [u32; 2]) -> Vec<u32> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let columns = size[0].div_ceil(TILE);
    let rows = size[1].div_ceil(TILE);
    let count = (columns * rows) as usize;
    let mut counts = vec![0u32; count];
    let mut local = Vec::new();
    let mut global = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let centre = glam::Vec2::from(node.centre);
        // One pixel of slack keeps floating-point projection error from
        // excluding a fragment on a tile boundary. The shader still evaluates
        // the original analytic field, so this adds candidates, not light.
        let radius = node.mark[1] + 1.0;
        let lo = ((centre - radius) / TILE as f32).floor().max(glam::Vec2::ZERO);
        let hi = ((centre + radius) / TILE as f32).floor() + 1.0;
        let bounds = [
            (lo.x as u32).min(columns),
            (lo.y as u32).min(rows),
            (hi.x.max(0.0) as u32).min(columns),
            (hi.y.max(0.0) as u32).min(rows),
        ];
        if (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]) > MAX_LOCAL_TILES {
            global.push(i as u32);
        } else {
            visit(bounds, columns, centre, radius, |tile| counts[tile] += 1);
            local.push((i as u32, bounds, centre, radius));
        }
    }
    let mut packed = Vec::with_capacity(4 + count + nodes.len());
    packed.extend([columns, 0, 0]);
    let mut end = (4 + count) as u32;
    for n in counts {
        packed.push(end);
        end += n;
    }
    packed.push(end);
    let mut cursors = packed[3..3 + count].to_vec();
    packed.resize(end as usize, 0);
    for (i, bounds, centre, radius) in local {
        visit(bounds, columns, centre, radius, |tile| {
            packed[cursors[tile] as usize] = i;
            cursors[tile] += 1;
        });
    }
    packed[1] = end;
    packed.extend(global);
    packed[2] = packed.len() as u32;
    packed
}

fn visit(
    bounds: [u32; 4],
    columns: u32,
    centre: glam::Vec2,
    radius: f32,
    mut at: impl FnMut(usize),
) {
    for y in bounds[1]..bounds[3] {
        for x in bounds[0]..bounds[2] {
            let lo = glam::vec2(x as f32, y as f32) * TILE as f32;
            let nearest = centre.clamp(lo, lo + TILE as f32);
            if nearest.distance_squared(centre) <= radius * radius {
                at((y * columns + x) as usize);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_window_of_small_glows_only_visits_local_nodes() {
        let mut nodes = Vec::new();
        for y in 0..64 {
            for x in 0..64 {
                nodes.push(GpuGlowNode {
                    centre: [(x * TILE + TILE / 2) as f32, (y * TILE + TILE / 2) as f32],
                    mark: [0.0, 3.0],
                    ..bytemuck::Zeroable::zeroed()
                });
            }
        }
        let packed = pack(&nodes, [2048, 2048]);
        assert_eq!(nodes.len(), 4096, "reach the live glow-row limit");
        assert_eq!(packed[1], packed[2], "small halos must not enter the global list");
        for tile in 0..4096 {
            assert_eq!(packed[4 + tile] - packed[3 + tile], 1);
            assert_eq!(packed[packed[3 + tile] as usize], tile as u32);
        }
        // Full-screen halos must not allocate tile_count * node_count indices.
        for node in &mut nodes {
            node.mark[1] = 10000.0;
        }
        let packed = pack(&nodes, [2048, 2048]);
        assert_eq!(packed.len(), 4 + 4096 + 4096);
        assert_eq!(packed[2] - packed[1], 4096);
    }
}
