// Copyright (c) 2025, Nils Hasenbanck
// Copyright (c) 2016, Intel Corporation
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated
// documentation files (the "Software"), to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to
// permit persons to whom the Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all copies or substantial portions of
// the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
// THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
// TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

struct Offset {
    block: u32
}

struct Settings {
    slow_mode: u32,
    fast_mode: u32,
    refine_iterations_1p: i32,
    refine_iterations_2p: i32,
    fast_skip_threshold: i32,
}

struct State {
    data: array<u32, 5>,
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> block_buffer: array<u32>;
@group(0) @binding(2) var<uniform> offsets_buffer: Offset;
@group(0) @binding(3) var<storage, read> settings: Settings;

var<private> block: array<f32, 64>;
var<private> state: State;

fn sq(x: f32) -> f32 {
    return x * x;
}

fn rsqrt(x: f32) -> f32 {
    return 1.0 / sqrt(x);
}

fn rcp(x: f32) -> f32 {
    return 1.0 / x;
}

@compute
@workgroup_size(8, 8)
fn compress_bc6h(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let xx = global_id.x;
    let yy = global_id.y;

    let texture_dimensions: vec2<u32> = textureDimensions(source_texture);

    let block_width = (texture_dimensions.x + 3u) / 4u;
    let block_height = (texture_dimensions.y + 3u) / 4u;

    if (xx >= block_width || yy >= block_height) {
        return;
    }

    var compressed_data: array<u32, 4>;

    // TODO: NHA implement BC6H
}
