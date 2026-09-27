    if research_complete_layer(in.layer) {
        for (var y = -1; y <= 1; y += 1) {
            let row = index + y * s.grid.x;
            let fy = f.y - f32(y);
            halo += research_complete_star(s, vec2<f32>(f.x + 1.0, fy), row - 1);
            halo += research_complete_star(s, vec2<f32>(f.x, fy), row);
            halo += research_complete_star(s, vec2<f32>(f.x - 1.0, fy), row + 1);
        }
        return halo;
    }
