// Copyright (c) 2025, Nils Hasenbanck
// Copyright (c) 2016-2024, Intel Corporation
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
    blocks_offset: u32,
}

struct Settings {
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

struct State {
    data: array<u32, 5>,
    best_err: f32,
    opaque_err: f32,
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> block_buffer: array<u32>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;
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

fn store_data(block_width: u32, xx: u32, yy: u32) {
    let offset = uniforms.blocks_offset + (yy * block_width * 4u + xx * 4u);

    block_buffer[offset + 0] = state.data[0];
    block_buffer[offset + 1] = state.data[1];
    block_buffer[offset + 2] = state.data[2];
    block_buffer[offset + 3] = state.data[3];
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

// Principal Component Analysis (PCA) bound
fn get_pca_bound(covar: array<f32, 10>, channels: u32) -> f32 {
    let power_iterations = 4u; // Quite approximative, but enough for bounding

    var covar_scaled = covar;
    let inv_var = 1.0 / (256.0 * 256.0);
    for (var k = 0u; k < 10u; k++) {
        covar_scaled[k] *= inv_var;
    }

    let eps = sq(0.001);
    covar_scaled[0] += eps;
    covar_scaled[4] += eps;
    covar_scaled[7] += eps;

    var axis: array<f32, 4>;
    compute_axis(&axis, &covar_scaled, power_iterations, channels);

    var a_vec: array<f32, 4>;
    if (channels == 3u) {
        ssymv3(&a_vec, &covar_scaled, &axis);
    } else if (channels == 4u) {
        ssymv4(&a_vec, &covar_scaled, &axis);
    }

    var sq_sum = 0.0;
    for (var p = 0u; p < channels; p++) {
        sq_sum += sq(a_vec[p]);
    }
    let lambda = sqrt(sq_sum);

    var bound = covar_scaled[0] + covar_scaled[4] + covar_scaled[7];
    if (channels == 4u) {
        bound += covar_scaled[9];
    }
    bound -= lambda;
    bound = max(bound, 0.0);

    return bound;
}

fn ssymv3(a: ptr<function, array<f32, 4>>, covar: ptr<function, array<f32, 10>>, b: ptr<function, array<f32, 4>>) {
    (*a)[0] = (*covar)[0] * (*b)[0] + (*covar)[1] * (*b)[1] + (*covar)[2] * (*b)[2];
    (*a)[1] = (*covar)[1] * (*b)[0] + (*covar)[4] * (*b)[1] + (*covar)[5] * (*b)[2];
    (*a)[2] = (*covar)[2] * (*b)[0] + (*covar)[5] * (*b)[1] + (*covar)[7] * (*b)[2];
}

fn ssymv4(a: ptr<function, array<f32, 4>>, covar: ptr<function, array<f32, 10>>, b: ptr<function, array<f32, 4>>) {
    (*a)[0] = (*covar)[0] * (*b)[0] + (*covar)[1] * (*b)[1] + (*covar)[2] * (*b)[2] + (*covar)[3] * (*b)[3];
    (*a)[1] = (*covar)[1] * (*b)[0] + (*covar)[4] * (*b)[1] + (*covar)[5] * (*b)[2] + (*covar)[6] * (*b)[3];
    (*a)[2] = (*covar)[2] * (*b)[0] + (*covar)[5] * (*b)[1] + (*covar)[7] * (*b)[2] + (*covar)[8] * (*b)[3];
    (*a)[3] = (*covar)[3] * (*b)[0] + (*covar)[6] * (*b)[1] + (*covar)[8] * (*b)[2] + (*covar)[9] * (*b)[3];
}

fn compute_axis(axis: ptr<function, array<f32, 4>>, covar: ptr<function, array<f32, 10>>, power_iterations: u32, channels: u32) {
    var a_vec: array<f32, 4> = array<f32, 4>(1.0, 1.0, 1.0, 1.0);

    for (var i = 0u; i < power_iterations; i++) {
        if (channels == 3u) {
            ssymv3(axis, covar, &a_vec);
        } else if (channels == 4u) {
            ssymv4(axis, covar, &a_vec);
        }

        for (var p = 0u; p < channels; p++) {
            a_vec[p] = (*axis)[p];
        }

        // Renormalize every other iteration
        if (i % 2u == 1u) {
            var norm_sq = 0.0;
            for (var p = 0u; p < channels; p++) {
                norm_sq += (*axis)[p] * (*axis)[p];
            }

            let rnorm = rsqrt(norm_sq);
            for (var p = 0u; p < channels; p++) {
                a_vec[p] *= rnorm;
            }
        }
    }

    for (var p = 0u; p < channels; p++) {
        (*axis)[p] = a_vec[p];
    }
}

fn compute_stats_masked(mask: u32, channels: u32) -> array<f32, 15> {
    var stats: array<f32, 15>;

    var mask_shifted = mask << 1u;
    for (var k = 0u; k < 16u; k++) {
        mask_shifted = mask_shifted >> 1u;
        let flag = f32(mask_shifted & 1u);

        var rgba: array<f32, 4>;
        for (var p = 0u; p < channels; p++) {
            rgba[p] = block[k + p * 16u] * flag;
        }
        stats[14] += flag;

        stats[10] += rgba[0];
        stats[11] += rgba[1];
        stats[12] += rgba[2];

        stats[0] += rgba[0] * rgba[0];
        stats[1] += rgba[0] * rgba[1];
        stats[2] += rgba[0] * rgba[2];

        stats[4] += rgba[1] * rgba[1];
        stats[5] += rgba[1] * rgba[2];

        stats[7] += rgba[2] * rgba[2];

        if (channels == 4u) {
            stats[13] += rgba[3];
            stats[3] += rgba[0] * rgba[3];
            stats[6] += rgba[1] * rgba[3];
            stats[8] += rgba[2] * rgba[3];
            stats[9] += rgba[3] * rgba[3];
        }
    }

    return stats;
}

fn covar_from_stats(stats: array<f32, 15>, channels: u32) -> array<f32, 10> {
    var covar: array<f32, 10>;

    covar[0] = stats[0] - stats[10] * stats[10] / stats[14];
    covar[1] = stats[1] - stats[10] * stats[11] / stats[14];
    covar[2] = stats[2] - stats[10] * stats[12] / stats[14];

    covar[4] = stats[4] - stats[11] * stats[11] / stats[14];
    covar[5] = stats[5] - stats[11] * stats[12] / stats[14];

    covar[7] = stats[7] - stats[12] * stats[12] / stats[14];

    if (channels == 4u) {
        covar[3] = stats[3] - stats[10] * stats[13] / stats[14];
        covar[6] = stats[6] - stats[11] * stats[13] / stats[14];
        covar[8] = stats[8] - stats[12] * stats[13] / stats[14];
        covar[9] = stats[9] - stats[13] * stats[13] / stats[14];
    }

    return covar;
}

fn block_pca_bound(mask: u32, channels: u32) -> f32 {
    let stats = compute_stats_masked(mask, channels);
    let covar = covar_from_stats(stats, channels);
    return get_pca_bound(covar, channels);
}

fn block_pca_bound_split(mask: u32, full_stats: array<f32, 15>, channels: u32) -> f32 {
    var stats = compute_stats_masked(mask, channels);

    let covar1 = covar_from_stats(stats, channels);

    for (var i = 0u; i < 15u; i++) {
        stats[i] = full_stats[i] - stats[i];
    }

    let covar2 = covar_from_stats(stats, channels);

    var bound = 0.0;
    bound += get_pca_bound(covar1, channels);
    bound += get_pca_bound(covar2, channels);

    return sqrt(bound) * 256.0;
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

fn put_bits(pos: ptr<function, u32>, bits: u32, v: u32) {
    state.data[(*pos) / 32u] |= v << ((*pos) % 32u);
    if ((*pos) % 32u + bits > 32u) {
        state.data[(*pos) / 32u + 1u] |= v >> (32u - (*pos) % 32u);
    }
    *pos += bits;
}

fn data_shl_1bit_from(from_bits: u32) {
    if (from_bits < 96u) {
        let shifted = (state.data[2] >> 1u) | (state.data[3] << 31u);
        let mask = ((1u << (from_bits - 64u)) - 1u) >> 1u;
        state.data[2] = (mask & state.data[2]) | (~mask & shifted);
        state.data[3] = (state.data[3] >> 1u) | (state.data[4] << 31u);
        state.data[4] = state.data[4] >> 1u;
    } else if (from_bits < 128u) {
        let shifted = (state.data[3] >> 1u) | (state.data[4] << 31u);
        let mask = ((1u << (from_bits - 96u)) - 1u) >> 1u;
        state.data[3] = (mask & state.data[3]) | (~mask & shifted);
        state.data[4] = state.data[4] >> 1u;
    }
}

fn bc7_code_qblock(qpos: ptr<function, u32>, qblock: ptr<function, array<u32, 2>>, bits: u32, flips: u32) {
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
                put_bits(qpos, bits - 1u, q);
            } else {
                put_bits(qpos, bits, q);
            }
            qbits_shifted >>= 4u;
            flips_shifted >>= 1u;
        }
    }
}

fn bc7_code_adjust_skip_mode01237(mode: u32, part_id: i32) {
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
        data_shl_1bit_from(128u + (pairs - 1u) - (15u - k) * bits);
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

fn bc7_code_mode01237(qep: ptr<function, array<i32, 24>>, qblock: ptr<function, array<u32, 2>>, part_id: i32, mode: u32) {
    let bits = select(2u, 3u, mode == 0u || mode == 1u);
    let pairs = select(2u, 3u, mode == 0u || mode == 2u);
    let channels = select(3u, 4u, mode == 7u);

    let flips = bc7_code_apply_swap_mode01237(qep, qblock, mode, part_id);

    for (var k = 0u; k < 5u; k++) {
        state.data[k] = 0u;
    }

    var pos = 0u;

    // Mode 0-3, 7
    put_bits(&pos, mode + 1u, 1u << mode);

    // Partition
    if (mode == 0u) {
        put_bits(&pos, 4u, u32(part_id & 15));
    } else {
        put_bits(&pos, 6u, u32(part_id & 63));
    }

    // Endpoints
    for (var p = 0u; p < channels; p++) {
        for (var j = 0u; j < pairs * 2u; j++) {
            if (mode == 0u) {
                put_bits(&pos, 4u, u32((*qep)[j * 4u + p]) >> 1u);
            } else if (mode == 1u) {
                put_bits(&pos, 6u, u32((*qep)[j * 4u + p]) >> 1u);
            } else if (mode == 2u) {
                put_bits(&pos, 5u, u32((*qep)[j * 4u + p]));
            } else if (mode == 3u) {
                put_bits(&pos, 7u, u32((*qep)[j * 4u + p]) >> 1u);
            } else if (mode == 7u) {
                put_bits(&pos, 5u, u32((*qep)[j * 4u + p]) >> 1u);
            }
        }
    }

    // P bits
    if (mode == 1u) {
        for (var j = 0u; j < 2u; j++) {
            put_bits(&pos, 1u, u32((*qep)[j * 8u]) & 1u);
        }
    }

    if (mode == 0u || mode == 3u || mode == 7u) {
        for (var j = 0u; j < pairs * 2u; j++) {
            put_bits(&pos, 1u, u32((*qep)[j * 4u]) & 1u);
        }
    }

    // Quantized values
    bc7_code_qblock(&pos, qblock, bits, flips);
    bc7_code_adjust_skip_mode01237(mode, part_id);
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
        //block_segment(&ep[j * 8], mask, channels);
    }

    // TODO
    //ep_quant_dequant(qep, &ep, mode, channels);

    // TODO
    //return block_quant(qblock, bits, &ep, pattern, channels);
    return 0.0;
}

fn bc7_enc_mode01237(mode: u32, part_list: array<i32, 64>, part_count: u32) {
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

    let refine_iterations = settings.refine_iterations[mode];
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
        best_err += state.opaque_err;
    }

    if (best_err < state.best_err) {
        state.best_err = best_err;
        bc7_code_mode01237(&best_qep, &best_qblock, best_part_id, mode);
    }
}

fn bc7_enc_mode02() {
    var part_list: array<i32, 64>;
    for (var part = 0; part < 64; part++) {
        part_list[part] = part;
    }

    bc7_enc_mode01237(0u, part_list, 16u);

    if (settings.skip_mode2 == 0) {
        bc7_enc_mode01237(2u, part_list, 64u);
    }
}

fn compress_block_bc7_core() {
    if (settings.mode_selection[0] != 0u) {
        bc7_enc_mode02();
    }
    if (settings.mode_selection[1] != 0u) {
        // TODO
        //bc7_enc_mode13();
        //bc7_enc_mode7();
    }
    if (settings.mode_selection[2] != 0u) {
        // TODO
        //bc7_enc_mode45();
    }
    if (settings.mode_selection[3] != 0u) {
        // TODO
        //bc7_enc_mode6();
    }
}

fn compute_opaque_err() -> f32 {
    if (settings.channels == 3u) {
        return 0.0;
    } else {
        var err = 0.0;
        for (var k = 0u; k < 16u; k++) {
            err += sq(block[48u + k] - 255.0);
        }
        return err;
    }
}

@compute
@workgroup_size(8, 8)
fn compress_bc7(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let xx = global_id.x;
    let yy = global_id.y;

    let block_width = (uniforms.width + 3u) / 4u;
    let block_height = (uniforms.height + 3u) / 4u;

    if (xx >= block_width || yy >= block_height) {
        return;
    }

    load_block_interleaved_rgba(xx, yy);

    state.best_err = 3.40282347e38;
    state.opaque_err = compute_opaque_err();

    compress_block_bc7_core();

    store_data(block_width, xx, yy);
}
