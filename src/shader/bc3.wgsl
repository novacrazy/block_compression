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

struct Uniforms {
    width: u32,
    height: u32,
    stride: u32,
}

@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

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

fn load_block_interleaved_rgba(xx: u32, yy: u32) {
    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            let src_offset = (yy * 4u + y) * uniforms.stride / 4u;
            let rgba = src[src_offset + xx * 4u + x];

            block[16u * 0u + y * 4u + x] = f32((rgba >>  0u) & 255u);
            block[16u * 1u + y * 4u + x] = f32((rgba >>  8u) & 255u);
            block[16u * 2u + y * 4u + x] = f32((rgba >> 16u) & 255u);
            block[16u * 3u + y * 4u + x] = f32((rgba >> 24u) & 255u);
        }
    }
}

fn store_data(xx: u32, yy: u32, data: array<u32, 4>) {
    let blocks_per_row = (uniforms.width + 3u) / 4u;
    let dst_offset = (yy * blocks_per_row + xx) * 4u;

    for (var k = 0u; k < 4u; k++) {
        dst[dst_offset + k] = data[k];
    }
}

fn compute_covar_dc_ugly(
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

fn stb__As16Bit(r: i32, g: i32, b: i32) -> i32 {
    return ((r >> 3) << 11) + ((g >> 2) << 5) + (b >> 3);
}

fn enc_rgb565(c: ptr<function, array<f32, 3>>) -> i32 {
    return stb__As16Bit(
        i32((*c)[0]),
        i32((*c)[1]),
        i32((*c)[2])
    );
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
    compute_covar_dc_ugly(&covar, &dc);

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
fn compress_bc3(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let xx = global_id.x;
    let yy = global_id.y;

    let block_width = (uniforms.width + 3u) / 4u;
    let block_height = (uniforms.height + 3u) / 4u;

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

    store_data(xx, yy, compressed_data);
}
