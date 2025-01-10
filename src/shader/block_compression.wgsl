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

struct BC6HSettings {
    slow_mode: u32,
    fast_mode: u32,
    refine_iterations_1p: i32,
    refine_iterations_2p: i32,
    fast_skip_threshold: i32,
}

struct BC7Settings {
    refine_iterations: array<u32, 8>,
    mode_selection: array<u32, 4>,
    skip_mode2: u32,
    fast_skip_threshold_mode1: u32,
    fast_skip_threshold_mode3: u32,
    fast_skip_threshold_mode7: u32,
    mode45_channel0: u32,
    refine_iterations_channel: u32,
    channels: u32,
}

struct BC7EncodingState {
    best_data: array<u32, 5>,
    best_err: f32,
    opaque_err: f32,
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> output_buffer: array<u32>;
@group(0) @binding(2) var<storage, read> bc6h_settings: BC6HSettings;
@group(0) @binding(3) var<storage, read> bc7_settings: BC7Settings;

var<private> block: array<f32, 64>;

fn sq(x: f32) -> f32 {
    return x * x;
}

fn rsqrt(x: f32) -> f32 {
    return 1.0 / sqrt(x);
}

fn rcp(x: f32) -> f32 {
    return 1.0 / x;
}

// Data is stored planar:
// RRRR|RRRR|RRRR|RRRR
// GGGG|GGGG|GGGG|GGGG
// BBBB|BBBB|BBBB|BBBB
// AAAA|AAAA|AAAA|AAAA
fn load_block_interleaved_rgba(xx: u32, yy: u32) {
    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            let pixel_x = xx * 4u + x;
            let pixel_y = yy * 4u + y;
            let rgba = textureLoad(source_texture, vec2<u32>(pixel_x, pixel_y), 0);

            block[16u * 0u + y * 4u + x] = rgba.r * 255.0;
            block[16u * 1u + y * 4u + x] = rgba.g * 255.0;
            block[16u * 2u + y * 4u + x] = rgba.b * 255.0;
            block[16u * 3u + y * 4u + x] = rgba.a * 255.0;
        }
    }
}

fn load_block_r_8bit(xx: u32, yy: u32) {
    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            let pixel_x = xx * 4u + x;
            let pixel_y = yy * 4u + y;
            let red = textureLoad(source_texture, vec2<u32>(pixel_x, pixel_y), 0).r;

            block[48u + y * 4u + x] = red * 255.0;
        }
    }
}

fn load_block_g_8bit(xx: u32, yy: u32) {
    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            let pixel_x = xx * 4u + x;
            let pixel_y = yy * 4u + y;
            let green = textureLoad(source_texture, vec2<u32>(pixel_x, pixel_y), 0).g;

            block[48u + y * 4u + x] = green * 255.0;
        }
    }
}

fn load_block_alpha_4bit(xx: u32, yy: u32) -> array<u32, 2> {
    var alpha_bits: array<u32, 2>;

    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            let pixel_x = xx * 4u + x;
            let pixel_y = yy * 4u + y;
            let alpha = textureLoad(source_texture, vec2<u32>(pixel_x, pixel_y), 0).a;

            // Convert alpha to 4 bits (0-15)
            let alpha4 = u32(alpha * 15.0);
            let bit_position = y * 16u + x * 4u;

            if (bit_position < 32u) {
                alpha_bits[0] |= (alpha4 << bit_position);
            } else {
                alpha_bits[1] |= (alpha4 << (bit_position - 32u));
            }
        }
    }

    return alpha_bits;
}

fn store_data_2(block_width: u32, xx: u32, yy: u32, data: array<u32, 2>) {
    let offset = yy * block_width * 2u + xx * 2u;

    output_buffer[offset + 0] = data[0];
    output_buffer[offset + 1] = data[1];
}

fn store_data_4(block_width: u32, xx: u32, yy: u32, data: array<u32, 4>) {
    let offset = yy * block_width * 4u + xx * 4u;

    output_buffer[offset + 0] = data[0];
    output_buffer[offset + 1] = data[1];
    output_buffer[offset + 2] = data[2];
    output_buffer[offset + 3] = data[3];
}

fn compute_covar_dc(
    covar: ptr<function, array<f32, 6>>,
    dc: ptr<function, array<f32, 3>>,
) {
    for (var p = 0u; p < 3u; p++) {
        var acc = 0.0;
        for (var k = 0u; k < 16u; k++) {
            acc += block[k + p * 16u];
        }
        (*dc)[p] = acc / 16.0;
    }

    var covar0 = 0.0;
    var covar1 = 0.0;
    var covar2 = 0.0;
    var covar3 = 0.0;
    var covar4 = 0.0;
    var covar5 = 0.0;

    for (var k = 0u; k < 16u; k++) {
        let rgb0 = block[k + 0u * 16u] - (*dc)[0];
        let rgb1 = block[k + 1u * 16u] - (*dc)[1];
        let rgb2 = block[k + 2u * 16u] - (*dc)[2];

        covar0 += rgb0 * rgb0;
        covar1 += rgb0 * rgb1;
        covar2 += rgb0 * rgb2;
        covar3 += rgb1 * rgb1;
        covar4 += rgb1 * rgb2;
        covar5 += rgb2 * rgb2;
    }

    (*covar)[0] = covar0;
    (*covar)[1] = covar1;
    (*covar)[2] = covar2;
    (*covar)[3] = covar3;
    (*covar)[4] = covar4;
    (*covar)[5] = covar5;
}

fn ssymv(result: ptr<function, array<f32, 3>>, covar: ptr<function, array<f32, 6>>, a_vector: ptr<function, array<f32, 3>>) {
    (*result)[0] = (*covar)[0] * (*a_vector)[0] + (*covar)[1] * (*a_vector)[1] + (*covar)[2] * (*a_vector)[2];
    (*result)[1] = (*covar)[1] * (*a_vector)[0] + (*covar)[3] * (*a_vector)[1] + (*covar)[4] * (*a_vector)[2];
    (*result)[2] = (*covar)[2] * (*a_vector)[0] + (*covar)[4] * (*a_vector)[1] + (*covar)[5] * (*a_vector)[2];
}

fn compute_axis3(axis: ptr<function, array<f32, 3>>, covar: ptr<function, array<f32, 6>>, powerIterations: i32) {
    var a_vector: array<f32, 3> = array<f32, 3>(1.0, 1.0, 1.0);

    for (var i = 0; i < powerIterations; i++) {
        ssymv(axis, covar, &a_vector);

        for (var p = 0u; p < 3u; p++) {
            a_vector[p] = (*axis)[p];
        }

        if (i % 2 == 1) {
            var norm_sq = 0.0;
            for (var p = 0u; p < 3u; p++) {
                norm_sq += (*axis)[p] * (*axis)[p];
            }

            let rnorm = rsqrt(norm_sq);
            for (var p = 0u; p < 3u; p++) {
                a_vector[p] *= rnorm;
            }
        }
    }

    for (var p = 0u; p < 3u; p++) {
        (*axis)[p] = a_vector[p];
    }
}

fn pick_endpoints(
    c0: ptr<function, array<f32, 3>>,
    c1: ptr<function, array<f32, 3>>,
    axis: ptr<function, array<f32, 3>>,
    dc: ptr<function, array<f32, 3>>
) {
    var min_dot = 256.0 * 256.0;
    var max_dot = 0.0;

    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            var dot = 0.0;
            for (var p = 0u; p < 3u; p++) {
                dot += (block[p * 16u + y * 4u + x] - (*dc)[p]) * (*axis)[p];
            }

            min_dot = min(min_dot, dot);
            max_dot = max(max_dot, dot);
        }
    }

    if (max_dot - min_dot < 1.0) {
        min_dot -= 0.5;
        max_dot += 0.5;
    }

    var norm_sq = 0.0;
    for (var p = 0u; p < 3u; p++) {
        norm_sq += (*axis)[p] * (*axis)[p];
    }

    let rnorm_sq = rcp(norm_sq);
    for (var p = 0u; p < 3u; p++) {
        (*c0)[p] = clamp((*dc)[p] + min_dot * rnorm_sq * (*axis)[p], 0.0, 255.0);
        (*c1)[p] = clamp((*dc)[p] + max_dot * rnorm_sq * (*axis)[p], 0.0, 255.0);
    }
}

fn dec_rgb565(c: ptr<function, array<f32, 3>>, p: i32) {
    let c2 = (p >> 0) & 31;
    let c1 = (p >> 5) & 63;
    let c0 = (p >> 11) & 31;

    (*c)[0] = f32((c0 << 3) + (c0 >> 2));
    (*c)[1] = f32((c1 << 2) + (c1 >> 4));
    (*c)[2] = f32((c2 << 3) + (c2 >> 2));
}

fn enc_rgb565(c: ptr<function, array<f32, 3>>) -> i32 {
    let r = i32((*c)[0]);
    let g = i32((*c)[1]);
    let b = i32((*c)[2]);

    return ((r >> 3) << 11) + ((g >> 2) << 5) + (b >> 3);
}

fn fast_quant(p0: i32, p1: i32) -> u32 {
    var c0: array<f32, 3>;
    var c1: array<f32, 3>;
    dec_rgb565(&c0, p0);
    dec_rgb565(&c1, p1);

    var dir: array<f32, 3>;
    for (var p = 0u; p < 3u; p++) {
        dir[p] = c1[p] - c0[p];
    }

    var sq_norm = 0.0;
    for (var p = 0u; p < 3u; p++) {
        sq_norm += sq(dir[p]);
    }

    let rsq_norm = rcp(sq_norm);

    for (var p = 0u; p < 3u; p++) {
        dir[p] *= rsq_norm * 3.0;
    }

    var bias = 0.5;
    for (var p = 0u; p < 3u; p++) {
        bias -= c0[p] * dir[p];
    }

    var bits = 0u;
    var scaler = 1u;
    for (var k = 0u; k < 16u; k++) {
        var dot = 0.0;
        for (var p = 0u; p < 3u; p++) {
            dot += block[k + p * 16u] * dir[p];
        }

        let q = clamp(i32(dot + bias), 0, 3);
        bits += u32(q) * scaler;
        scaler *= 4u;
    }

    return bits;
}

fn bc1_refine(pe: ptr<function, array<i32, 2>>, bits: u32, dc: ptr<function, array<f32, 3>>) {
    var c0: array<f32, 3>;
    var c1: array<f32, 3>;

    if ((bits ^ (bits * 4u)) < 4u) {
        for (var p = 0u; p < 3u; p++) {
            c0[p] = (*dc)[p];
            c1[p] = (*dc)[p];
        }
    } else {
        var atb1: array<f32, 3> = array<f32, 3>(0.0, 0.0, 0.0);
        var sum_q = 0.0;
        var sum_qq = 0.0;
        var shifted_bits = bits;

        for (var k = 0u; k < 16u; k++) {
            let q = f32(shifted_bits & 3u);
            shifted_bits = shifted_bits >> 2u;

            let x = 3.0 - q;
            let y = q;

            sum_q += q;
            sum_qq += q * q;

            for (var p = 0u; p < 3u; p++) {
                atb1[p] += x * block[k + p * 16u];
            }
        }

        var sum: array<f32, 3>;
        var atb2: array<f32, 3>;

        for (var p = 0u; p < 3u; p++) {
            sum[p] = (*dc)[p] * 16.0;
            atb2[p] = 3.0 * sum[p] - atb1[p];
        }

        let cxx = 16.0 * sq(3.0) - 2.0 * 3.0 * sum_q + sum_qq;
        let cyy = sum_qq;
        let cxy = 3.0 * sum_q - sum_qq;
        let scale = 3.0 * rcp(cxx * cyy - cxy * cxy);

        for (var p = 0u; p < 3u; p++) {
            c0[p] = (atb1[p] * cyy - atb2[p] * cxy) * scale;
            c1[p] = (atb2[p] * cxx - atb1[p] * cxy) * scale;

            c0[p] = clamp(c0[p], 0.0, 255.0);
            c1[p] = clamp(c1[p], 0.0, 255.0);
        }
    }

    (*pe)[0] = enc_rgb565(&c0);
    (*pe)[1] = enc_rgb565(&c1);
}

fn fix_qbits(qbits: u32) -> u32 {
    let mask_01b: u32 = 0x55555555u;
    let mask_10b: u32 = 0xAAAAAAAAu;

    let qbits0 = qbits & mask_01b;
    let qbits1 = qbits & mask_10b;
    return (qbits1 >> 1u) + (qbits1 ^ (qbits0 << 1u));
}

fn compress_block_bc1_core() -> array<u32, 2> {
    let power_iterations = 4;
    let refine_iterations = 1;

    var covar: array<f32, 6>;
    var dc: array<f32, 3>;
    compute_covar_dc(&covar, &dc);

    let eps = 0.001;
    covar[0] += eps;
    covar[3] += eps;
    covar[5] += eps;

    var axis: array<f32, 3>;
    compute_axis3(&axis, &covar, power_iterations);

    var c0: array<f32, 3>;
    var c1: array<f32, 3>;
    pick_endpoints(&c0, &c1, &axis, &dc);

    var p: array<i32, 2>;
    p[0] = enc_rgb565(&c0);
    p[1] = enc_rgb565(&c1);
    if (p[0] < p[1]) {
        let temp = p[0];
        p[0] = p[1];
        p[1] = temp;
    }

    var data: array<u32, 2>;
    data[0] = (u32(p[1]) << 16u) | u32(p[0]);
    data[1] = fast_quant(p[0], p[1]);

    for (var i = 0; i < refine_iterations; i++) {
        bc1_refine(&p, data[1], &dc);
        if (p[0] < p[1]) {
            let temp = p[0];
            p[0] = p[1];
            p[1] = temp;
        }
        data[0] = (u32(p[1]) << 16u) | u32(p[0]);
        data[1] = fast_quant(p[0], p[1]);
    }

    data[1] = fix_qbits(data[1]);
    return data;
}

fn compress_block_bc3_alpha() -> array<u32, 2> {
    var ep: array<f32, 2> = array<f32, 2>(255.0, 0.0);

    // Find min/max endpoints using block[48] to block[63] for alpha
    for (var k: u32 = 0u; k < 16u; k++) {
        ep[0] = min(ep[0], block[48 + k]);
        ep[1] = max(ep[1], block[48 + k]);
    }

    // Prevent division by zero
    if (ep[0] == ep[1]) {
        ep[1] = ep[0] + 0.1;
    }

    var qblock: array<u32, 2> = array<u32, 2>(0u, 0u);
    let scale = 7.0 / (ep[1] - ep[0]);

    for (var k: u32 = 0u; k < 16u; k++) {
        let v = block[48u + k];
        let proj = (v - ep[0]) * scale + 0.5;

        var q = clamp(i32(proj), 0, 7);
        q = 7 - q;

        if (q > 0) {
            q += 1;
        }
        if (q == 8) {
            q = 1;
        }

        qblock[k / 8u] |= u32(q) << ((k % 8u) * 3u);
    }

    var data: array<u32, 2>;
    data[0] = (clamp(u32(ep[0]), 0u, 255u) << 8u) | clamp(u32(ep[1]), 0u, 255u);
    data[0] |= qblock[0] << 16u;
    data[1] = qblock[0] >> 16u;
    data[1] |= qblock[1] << 8u;

    return data;
}

@compute
@workgroup_size(8, 8)
fn compress_bc1(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let xx = global_id.x;
    let yy = global_id.y;

    let texture_dimensions: vec2<u32> = textureDimensions(source_texture);

    let block_width = (texture_dimensions.x + 3u) / 4u;
    let block_height = (texture_dimensions.y + 3u) / 4u;

    if (xx >= block_width || yy >= block_height) {
        return;
    }

    load_block_interleaved_rgba(xx, yy);
    var compressed_data: array<u32, 2>;

    let color_result = compress_block_bc1_core();
    compressed_data[0] = color_result[0];
    compressed_data[1] = color_result[1];

    store_data_2(block_width, xx, yy, compressed_data);
}

@compute
@workgroup_size(8, 8)
fn compress_bc2(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let xx = global_id.x;
    let yy = global_id.y;

    let texture_dimensions: vec2<u32> = textureDimensions(source_texture);

    let block_width = (texture_dimensions.x + 3u) / 4u;
    let block_height = (texture_dimensions.y + 3u) / 4u;

    if (xx >= block_width || yy >= block_height) {
        return;
    }

    var compressed_data: array<u32, 4>;

    let alpha_result = load_block_alpha_4bit(xx, yy);
    compressed_data[0] = alpha_result[0];
    compressed_data[1] = alpha_result[1];

    load_block_interleaved_rgba(xx, yy);

    let color_result = compress_block_bc1_core();
    compressed_data[2] = color_result[0];
    compressed_data[3] = color_result[1];

    store_data_4(block_width, xx, yy, compressed_data);
}

@compute
@workgroup_size(8, 8)
fn compress_bc3(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let xx = global_id.x;
    let yy = global_id.y;

    let texture_dimensions: vec2<u32> = textureDimensions(source_texture);

    let block_width = (texture_dimensions.x + 3u) / 4u;
    let block_height = (texture_dimensions.y + 3u) / 4u;

    if (xx >= block_width || yy >= block_height) {
        return;
    }

    load_block_interleaved_rgba(xx, yy);
    var compressed_data: array<u32, 4>;

    let alpha_result = compress_block_bc3_alpha();
    compressed_data[0] = alpha_result[0];
    compressed_data[1] = alpha_result[1];

    let color_result = compress_block_bc1_core();
    compressed_data[2] = color_result[0];
    compressed_data[3] = color_result[1];

    store_data_4(block_width, xx, yy, compressed_data);
}

@compute
@workgroup_size(8, 8)
fn compress_bc4(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let xx = global_id.x;
    let yy = global_id.y;

    let texture_dimensions: vec2<u32> = textureDimensions(source_texture);

    let block_width = (texture_dimensions.x + 3u) / 4u;
    let block_height = (texture_dimensions.y + 3u) / 4u;

    if (xx >= block_width || yy >= block_height) {
        return;
    }

    load_block_r_8bit(xx, yy);
    var compressed_data: array<u32, 2>;

    let color_result = compress_block_bc3_alpha();
    compressed_data[0] = color_result[0];
    compressed_data[1] = color_result[1];

    store_data_2(block_width, xx, yy, compressed_data);
}

@compute
@workgroup_size(8, 8)
fn compress_bc5(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let xx = global_id.x;
    let yy = global_id.y;

    let texture_dimensions: vec2<u32> = textureDimensions(source_texture);

    let block_width = (texture_dimensions.x + 3u) / 4u;
    let block_height = (texture_dimensions.y + 3u) / 4u;

    if (xx >= block_width || yy >= block_height) {
        return;
    }

    var compressed_data: array<u32, 4>;

    load_block_r_8bit(xx, yy);
    let red_result = compress_block_bc3_alpha();
    compressed_data[0] = red_result[0];
    compressed_data[1] = red_result[1];

    load_block_g_8bit(xx, yy);
    let green_result = compress_block_bc3_alpha();
    compressed_data[2] = green_result[0];
    compressed_data[3] = green_result[1];

    store_data_4(block_width, xx, yy, compressed_data);
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

fn get_unquant_table(bits: u32) -> array<u32, 16> {
    switch (bits) {
        case 2u: {
            return array<u32, 16>(
                0u, 21u, 43u, 64u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u
            );
        }
        case 3u: {
            return array<u32, 16>(
                0u, 9u, 18u, 27u, 37u, 46u, 55u, 64u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u
            );
        }
        default: {
            return array<u32, 16>(
                0u, 4u, 9u, 13u, 17u, 21u, 26u, 30u, 34u, 38u, 43u, 47u, 51u, 55u, 60u, 64u
            );
        }
    }
}

fn get_pattern(part_id: i32) -> u32 {
    let pattern_table = array<u32, 128>(
        0x50505050u, 0x40404040u, 0x54545454u, 0x54505040u, 0x50404000u, 0x55545450u, 0x55545040u, 0x54504000u,
		0x50400000u, 0x55555450u, 0x55544000u, 0x54400000u, 0x55555440u, 0x55550000u, 0x55555500u, 0x55000000u,
		0x55150100u, 0x00004054u, 0x15010000u, 0x00405054u, 0x00004050u, 0x15050100u, 0x05010000u, 0x40505054u,
		0x00404050u, 0x05010100u, 0x14141414u, 0x05141450u, 0x01155440u, 0x00555500u, 0x15014054u, 0x05414150u,
		0x44444444u, 0x55005500u, 0x11441144u, 0x05055050u, 0x05500550u, 0x11114444u, 0x41144114u, 0x44111144u,
		0x15055054u, 0x01055040u, 0x05041050u, 0x05455150u, 0x14414114u, 0x50050550u, 0x41411414u, 0x00141400u,
		0x00041504u, 0x00105410u, 0x10541000u, 0x04150400u, 0x50410514u, 0x41051450u, 0x05415014u, 0x14054150u,
		0x41050514u, 0x41505014u, 0x40011554u, 0x54150140u, 0x50505500u, 0x00555050u, 0x15151010u, 0x54540404u,
		0xAA685050u, 0x6A5A5040u, 0x5A5A4200u, 0x5450A0A8u, 0xA5A50000u, 0xA0A05050u, 0x5555A0A0u, 0x5A5A5050u,
		0xAA550000u, 0xAA555500u, 0xAAAA5500u, 0x90909090u, 0x94949494u, 0xA4A4A4A4u, 0xA9A59450u, 0x2A0A4250u,
		0xA5945040u, 0x0A425054u, 0xA5A5A500u, 0x55A0A0A0u, 0xA8A85454u, 0x6A6A4040u, 0xA4A45000u, 0x1A1A0500u,
		0x0050A4A4u, 0xAAA59090u, 0x14696914u, 0x69691400u, 0xA08585A0u, 0xAA821414u, 0x50A4A450u, 0x6A5A0200u,
		0xA9A58000u, 0x5090A0A8u, 0xA8A09050u, 0x24242424u, 0x00AA5500u, 0x24924924u, 0x24499224u, 0x50A50A50u,
		0x500AA550u, 0xAAAA4444u, 0x66660000u, 0xA5A0A5A0u, 0x50A050A0u, 0x69286928u, 0x44AAAA44u, 0x66666600u,
		0xAA444444u, 0x54A854A8u, 0x95809580u, 0x96969600u, 0xA85454A8u, 0x80959580u, 0xAA141414u, 0x96960000u,
		0xAAAA1414u, 0xA05050A0u, 0xA0A5A5A0u, 0x96000000u, 0x40804080u, 0xA9A8A9A8u, 0xAAAAAA44u, 0x2A4A5254u,
    );

    return pattern_table[part_id];
}

fn get_pattern_mask(part_id: i32, j: u32) -> u32 {
    let pattern_mask_table = array<u32, 128>(
		0xCCCC3333u, 0x88887777u, 0xEEEE1111u, 0xECC81337u, 0xC880377Fu, 0xFEEC0113u, 0xFEC80137u, 0xEC80137Fu,
		0xC80037FFu, 0xFFEC0013u, 0xFE80017Fu, 0xE80017FFu, 0xFFE80017u, 0xFF0000FFu, 0xFFF0000Fu, 0xF0000FFFu,
		0xF71008EFu, 0x008EFF71u, 0x71008EFFu, 0x08CEF731u, 0x008CFF73u, 0x73108CEFu, 0x3100CEFFu, 0x8CCE7331u,
		0x088CF773u, 0x3110CEEFu, 0x66669999u, 0x366CC993u, 0x17E8E817u, 0x0FF0F00Fu, 0x718E8E71u, 0x399CC663u,
		0xAAAA5555u, 0xF0F00F0Fu, 0x5A5AA5A5u, 0x33CCCC33u, 0x3C3CC3C3u, 0x55AAAA55u, 0x96966969u, 0xA55A5AA5u,
		0x73CE8C31u, 0x13C8EC37u, 0x324CCDB3u, 0x3BDCC423u, 0x69969669u, 0xC33C3CC3u, 0x99666699u, 0x0660F99Fu,
		0x0272FD8Du, 0x04E4FB1Bu, 0x4E40B1BFu, 0x2720D8DFu, 0xC93636C9u, 0x936C6C93u, 0x39C6C639u, 0x639C9C63u,
		0x93366CC9u, 0x9CC66339u, 0x817E7E81u, 0xE71818E7u, 0xCCF0330Fu, 0x0FCCF033u, 0x774488BBu, 0xEE2211DDu,
		0x08CC0133u, 0x8CC80037u, 0xCC80006Fu, 0xEC001331u, 0x330000FFu, 0x00CC3333u, 0xFF000033u, 0xCCCC0033u,
		0x0F0000FFu, 0x0FF0000Fu, 0x00F0000Fu, 0x44443333u, 0x66661111u, 0x22221111u, 0x136C0013u, 0x008C8C63u,
		0x36C80137u, 0x08CEC631u, 0x3330000Fu, 0xF0000333u, 0x00EE1111u, 0x88880077u, 0x22C0113Fu, 0x443088CFu,
		0x0C22F311u, 0x03440033u, 0x69969009u, 0x9960009Fu, 0x03303443u, 0x00660699u, 0xC22C3113u, 0x8C0000EFu,
		0x1300007Fu, 0xC4003331u, 0x004C1333u, 0x22229999u, 0x00F0F00Fu, 0x24929249u, 0x29429429u, 0xC30C30C3u,
		0xC03C3C03u, 0x00AA0055u, 0xAA0000FFu, 0x30300303u, 0xC0C03333u, 0x90900909u, 0xA00A5005u, 0xAAA0000Fu,
		0x0AAA0555u, 0xE0E01111u, 0x70700707u, 0x6660000Fu, 0x0EE01111u, 0x07707007u, 0x06660999u, 0x660000FFu,
		0x00660099u, 0x0CC03333u, 0x03303003u, 0x60000FFFu, 0x80807777u, 0x10100101u, 0x000A0005u, 0x08CE8421u,
    );

    let mask_packed = pattern_mask_table[part_id];
    let mask0 = mask_packed & 0xFFFFu;
    let mask1 = mask_packed >> 16u;

    return select(select(mask1, mask0, j == 0), ~mask0 & ~mask1, j == 2);
}

fn get_skips(part_id: i32) -> array<u32, 3> {
    let skip_table = array<u32, 128>(
        0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u,
        0xf0u, 0x20u, 0x80u, 0x20u, 0x20u, 0x80u, 0x80u, 0xf0u, 0x20u, 0x80u, 0x20u, 0x20u, 0x80u, 0x80u, 0x20u, 0x20u,
        0xf0u, 0xf0u, 0x60u, 0x80u, 0x20u, 0x80u, 0xf0u, 0xf0u, 0x20u, 0x80u, 0x20u, 0x20u, 0x20u, 0xf0u, 0xf0u, 0x60u,
        0x60u, 0x20u, 0x60u, 0x80u, 0xf0u, 0xf0u, 0x20u, 0x20u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0xf0u, 0x20u, 0x20u, 0xf0u,
        0x3fu, 0x38u, 0xf8u, 0xf3u, 0x8fu, 0x3fu, 0xf3u, 0xf8u, 0x8fu, 0x8fu, 0x6fu, 0x6fu, 0x6fu, 0x5fu, 0x3fu, 0x38u,
        0x3fu, 0x38u, 0x8fu, 0xf3u, 0x3fu, 0x38u, 0x6fu, 0xa8u, 0x53u, 0x8fu, 0x86u, 0x6au, 0x8fu, 0x5fu, 0xfau, 0xf8u,
		0x8fu, 0xf3u, 0x3fu, 0x5au, 0x6au, 0xa8u, 0x89u, 0xfau, 0xf6u, 0x3fu, 0xf8u, 0x5fu, 0xf3u, 0xf6u, 0xf6u, 0xf8u,
        0x3fu, 0xf3u, 0x5fu, 0x5fu, 0x5fu, 0x8fu, 0x5fu, 0xafu, 0x5fu, 0xafu, 0x8fu, 0xdfu, 0xf3u, 0xcfu, 0x3fu, 0x38u,
    );

    let skip_packed = skip_table[part_id];

    var skips: array<u32, 3>;
    skips[0] = 0u;
    skips[1] = skip_packed >> 4u;
    skips[2] = skip_packed & 15u;
    return skips;
}

fn partial_sort_list(list: ptr<function, array<u32, 64>>, length: u32, partial_count: u32) {
    for (var k = 0u; k < partial_count; k++) {
        var best_idx = k;
        var best_value = (*list)[k];

        for (var i = k + 1u; i < length; i++) {
            if (best_value > (*list)[i]) {
                best_value = (*list)[i];
                best_idx = i;
            }
        }

        let temp = (*list)[k];
        (*list)[k] = best_value;
        (*list)[best_idx] = temp;
    }
}

fn put_bits(data: ptr<function, array<u32, 5>>, pos: ptr<function, u32>, bits: u32, v: u32) {
    (*data)[(*pos) / 32u] |= v << ((*pos) % 32u);
    if ((*pos) % 32u + bits > 32u) {
        (*data)[(*pos) / 32u + 1u] |= v >> (32u - (*pos) % 32u);
    }
    *pos += bits;
}

fn data_shl_1bit_from(data: ptr<function, array<u32, 5>>, from_bits: u32) {
    if (from_bits < 96u) {
        let shifted = ((*data)[2] >> 1u) | ((*data)[3] << 31u);
        let mask = ((1u << (from_bits - 64u)) - 1u) >> 1u;
        (*data)[2] = (mask & (*data)[2]) | (~mask & shifted);
        (*data)[3] = ((*data)[3] >> 1u) | ((*data)[4] << 31u);
        (*data)[4] = (*data)[4] >> 1u;
    } else if (from_bits < 128u) {
        let shifted = ((*data)[3] >> 1u) | ((*data)[4] << 31u);
        let mask = ((1u << (from_bits - 96u)) - 1u) >> 1u;
        (*data)[3] = (mask & (*data)[3]) | (~mask & shifted);
        (*data)[4] = (*data)[4] >> 1u;
    }
}

fn bc7_code_qblock(data: ptr<function, array<u32, 5>>, qpos: ptr<function, u32>, qblock: ptr<function, array<u32, 2>>, bits: u32, flips: u32) {
    let levels = 1u << bits;
    var flips_shifted = flips;

    for (var k1 = 0u; k1 < 2u; k1++) {
        var qbits_shifted = (*qblock)[k1];
        for (var k2 = 0u; k2 < 8u; k2++) {
            var q = qbits_shifted & 15u;
            if ((flips_shifted & 1u) > 0u) {
                q = (levels - 1u) - q;
            }

            if (k1 == 0u && k2 == 0u) {
                put_bits(data, qpos, bits - 1u, q);
            } else {
                put_bits(data, qpos, bits, q);
            }
            qbits_shifted >>= 4u;
            flips_shifted >>= 1u;
        }
    }
}

fn bc7_code_adjust_skip_mode01237(data: ptr<function, array<u32, 5>>, mode: u32, part_id: i32) {
    let pairs = select(2u, 3u, mode == 0u || mode == 2u);
    let bits = select(2u, 3u, mode == 0u || mode == 1u);

    var skips = get_skips(part_id);

    if (pairs > 2u && skips[1] < skips[2]) {
        let t = skips[1];
        skips[1] = skips[2];
        skips[2] = t;
    }

    for (var j = 1u; j < pairs; j++) {
        let k = skips[j];
        data_shl_1bit_from(data, 128u + (pairs - 1u) - (15u - k) * bits);
    }
}

fn bc7_code_apply_swap_mode01237(qep: ptr<function, array<i32, 24>>, qblock: ptr<function, array<u32, 2>>, mode: u32, part_id: i32) -> u32 {
    let bits = select(2u, 3u, mode == 0u || mode == 1u);
    let pairs = select(2u, 3u, mode == 0u || mode == 2u);

    var flips = 0u;
    let levels = 1u << bits;

    let skips = get_skips(part_id);

    for (var j = 0u; j < pairs; j++) {
        let k0 = skips[j];
        // Extract 4 bits from qblock at position k0
        let q = ((*qblock)[k0 >> 3u] << (28u - (k0 & 7u) * 4u)) >> 28u;

        if (q >= levels / 2u) {
            for (var p = 0u; p < 4u; p++) {
                let temp = (*qep)[8u * j + p];
                (*qep)[8u * j + p] = (*qep)[8u * j + 4u + p];
                (*qep)[8u * j + 4u + p] = temp;
            }

            let pmask = get_pattern_mask(part_id, j);
            flips |= u32(pmask);
        }
    }

    return flips;
}

fn bc7_code_mode01237(data: ptr<function, array<u32, 5>>, qep: ptr<function, array<i32, 24>>, qblock: ptr<function, array<u32, 2>>, part_id: i32, mode: u32) {
    let bits = select(2u, 3u, mode == 0u || mode == 1u);
    let pairs = select(2u, 3u, mode == 0u || mode == 2u);
    let channels = select(3u, 4u, mode == 7u);

    let flips = bc7_code_apply_swap_mode01237(qep, qblock, mode, part_id);

    for (var k = 0u; k < 5u; k++) {
        (*data)[k] = 0u;
    }

    var pos = 0u;

    // Mode 0-3, 7
    put_bits(data, &pos, mode + 1u, 1u << mode);

    // Partition
    if (mode == 0u) {
        put_bits(data, &pos, 4u, u32(part_id & 15));
    } else {
        put_bits(data, &pos, 6u, u32(part_id & 63));
    }

    // Endpoints
    for (var p = 0u; p < channels; p++) {
        for (var j = 0u; j < pairs * 2u; j++) {
            if (mode == 0u) {
                put_bits(data, &pos, 4u, u32((*qep)[j * 4u + p]) >> 1u);
            } else if (mode == 1u) {
                put_bits(data, &pos, 6u, u32((*qep)[j * 4u + p]) >> 1u);
            } else if (mode == 2u) {
                put_bits(data, &pos, 5u, u32((*qep)[j * 4u + p]));
            } else if (mode == 3u) {
                put_bits(data, &pos, 7u, u32((*qep)[j * 4u + p]) >> 1u);
            } else if (mode == 7u) {
                put_bits(data, &pos, 5u, u32((*qep)[j * 4u + p]) >> 1u);
            }
        }
    }

    // P bits
    if (mode == 1u) {
        for (var j = 0u; j < 2u; j++) {
            put_bits(data, &pos, 1u, u32((*qep)[j * 8u]) & 1u);
        }
    }

    if (mode == 0u || mode == 3u || mode == 7u) {
        for (var j = 0u; j < pairs * 2u; j++) {
            put_bits(data, &pos, 1u, u32((*qep)[j * 4u]) & 1u);
        }
    }

    // Quantized values
    bc7_code_qblock(data, &pos, qblock, bits, flips);
    bc7_code_adjust_skip_mode01237(data, mode, part_id);
}

fn bc7_enc_mode01237_part_fast(qep: ptr<function, array<i32, 24>>, qblock: ptr<function, array<u32, 2>>, part_id: i32, mode: u32) -> f32 {
    let pattern = get_pattern(part_id);
    let bits = select(2u, 3u, mode == 0u || mode == 1u);
    let pairs = select(2u, 3u, mode == 0u || mode == 2u);
    let channels = select(3u, 4u, mode == 7u);

    var ep: array<f32, 24>;
    for (var j = 0u; j < pairs; j++) {
        let mask = get_pattern_mask(part_id, j);
        // TODO
        //block_segment(&ep[j * 8], block, mask, channels);
    }

    // TODO
    //ep_quant_dequant(qep, &ep, mode, channels);

    // TODO
    //return block_quant(qblock, bits, &ep, pattern, channels);
    return 0.0;
}

fn bc7_enc_mode01237(state: ptr<function, BC7EncodingState>, mode: u32, part_list: array<i32, 64>, part_count: u32) {
    if (part_count == 0u) {
        return;
    }

    let bits = select(2u, 3u, mode == 0u || mode == 1u);
    let pairs = select(2u, 3u, mode == 0u || mode == 2u);
    let channels = select(3u, 4u, mode == 7u);

    var best_qep: array<i32, 24>;
    var best_qblock: array<u32, 2>;
    var best_part_id = -1;
    var best_err = 3.40282347e38;

    for (var part = 0u; part < part_count; part++) {
        var part_id = part_list[part] & 63;
        part_id = select(part_id, part_id + 64, pairs == 3);

        var qep: array<i32, 24>;
        var qblock: array<u32, 2>;
        let err = bc7_enc_mode01237_part_fast(&qep, &qblock, part_id, mode);

        if (err < best_err) {
            for (var i = 0u; i < 8u * pairs; i++) {
                best_qep[i] = qep[i];
            }
            for (var k = 0u; k < 2u; k++) {
                best_qblock[k] = qblock[k];
            }

            best_part_id = part_id;
            best_err = err;
        }
    }

    let refine_iterations = bc7_settings.refine_iterations[mode];
    for (var i = 0u; i < refine_iterations; i++) {
        var ep: array<f32, 24>;
        for (var j = 0u; j < pairs; j++) {
            let mask = get_pattern_mask(best_part_id, j);
            // TODO
            //opt_endpoints(&ep[j * 8], bits, best_qblock, mask, channels);
        }

        var qep: array<i32, 24>;
        var qblock: array<u32, 2>;

        // TODO
        //ep_quant_dequant(&qep, &ep, mode, channels);

        let pattern = get_pattern(best_part_id);
        // TODO
        //let err = block_quant(&qblock, bits, ep, pattern, channels);
        let err = 0.0;

        if (err < best_err) {
            for (var i = 0u; i < 8u * pairs; i++) {
                best_qep[i] = qep[i];
            }
            for (var k = 0u; k < 2u; k++) {
                best_qblock[k] = qblock[k];
            }

            best_err = err;
        }
    }

    if (mode != 7u) {
        best_err += (*state).opaque_err;
    }

    if (best_err < (*state).best_err) {
        (*state).best_err = best_err;
        bc7_code_mode01237(&(*state).best_data, &best_qep, &best_qblock, best_part_id, mode);
    }
}

fn bc7_enc_mode02(state: ptr<function, BC7EncodingState>) {
    var part_list: array<i32, 64>;
    for (var part = 0; part < 64; part++) {
        part_list[part] = part;
    }

    bc7_enc_mode01237(state, 0u, part_list, 16u);

    if (!bc7_settings.skip_mode2) {
        bc7_enc_mode01237(state, 2u, part_list, 64u);
    }
}

fn compress_block_bc7_core(state: ptr<function, BC7EncodingState>) {
    if (bc7_settings.mode_selection[0] > 0u) {
        bc7_enc_mode02(state);
    }
    if (bc7_settings.mode_selection[1] > 0u) {
        // TODO
        //bc7_enc_mode13(state);
        //bc7_enc_mode7(state);
    }
    if (bc7_settings.mode_selection[2] > 0u) {
        // TODO
        //bc7_enc_mode45(state);
    }
    if (bc7_settings.mode_selection[3] > 0u) {
        // TODO
        //bc7_enc_mode6(state);
    }
}

fn compute_opaque_err() -> f32 {
    if (bc7_settings.channels == 3u) {
        return 0.0;
    }

    var err = 0.0;
    for (var k = 0u; k < 16u; k++) {
        err += sq(block[48u + k] - 255.0);
    }
    return err;
}

@compute
@workgroup_size(8, 8)
fn compress_bc7(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let xx = global_id.x;
    let yy = global_id.y;

    let texture_dimensions: vec2<u32> = textureDimensions(source_texture);
    let block_width = (texture_dimensions.x + 3u) / 4u;
    let block_height = (texture_dimensions.y + 3u) / 4u;

    if (xx >= block_width || yy >= block_height) {
        return;
    }

    load_block_interleaved_rgba(xx, yy);

    var state: BC7EncodingState;
	state.best_err = 3.40282347e38;
    state.opaque_err = compute_opaque_err();

    compress_block_bc7_core(&state);

    store_data_4(block_width, xx, yy, state.best_data);
}
