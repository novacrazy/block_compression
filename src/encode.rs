//! CPU based encoding.

use crate::CompressionVariant;

struct BlockCompressorBC15 {
    block: [f32; 64],
}

impl Default for BlockCompressorBC15 {
    fn default() -> Self {
        Self { block: [0.0; 64] }
    }
}

impl BlockCompressorBC15 {
    fn load_block_interleaved_rgba(
        &mut self,
        rgba_data: &[u8],
        xx: usize,
        yy: usize,
        stride: usize,
    ) {
        for y in 0..4 {
            for x in 0..4 {
                let pixel_x = xx * 4 + x;
                let pixel_y = yy * 4 + y;

                let offset = pixel_y * stride + pixel_x * 4;

                let red = rgba_data[offset] as f32;
                let green = rgba_data[offset + 1] as f32;
                let blue = rgba_data[offset + 2] as f32;
                let alpha = rgba_data[offset + 3] as f32;

                self.block[y * 4 + x] = red;
                self.block[16 + y * 4 + x] = green;
                self.block[32 + y * 4 + x] = blue;
                self.block[48 + y * 4 + x] = alpha;
            }
        }
    }

    fn load_block_r_8bit(&mut self, rgba_data: &[u8], xx: usize, yy: usize, stride: usize) {
        for y in 0..4 {
            for x in 0..4 {
                let pixel_x = xx * 4 + x;
                let pixel_y = yy * 4 + y;

                let offset = pixel_y * stride + pixel_x * 4;
                let red = rgba_data[offset] as f32;

                self.block[48 + y * 4 + x] = red;
            }
        }
    }

    fn load_block_g_8bit(&mut self, rgba_data: &[u8], xx: usize, yy: usize, stride: usize) {
        for y in 0..4 {
            for x in 0..4 {
                let pixel_x = xx * 4 + x;
                let pixel_y = yy * 4 + y;

                let offset = pixel_y * stride + pixel_x * 4;
                let green = rgba_data[offset + 1] as f32;

                self.block[48 + y * 4 + x] = green;
            }
        }
    }

    fn load_block_alpha_4bit(
        &mut self,
        rgba_data: &[u8],
        xx: usize,
        yy: usize,
        stride: usize,
    ) -> [u32; 2] {
        let mut alpha_bits = [0; 2];

        for y in 0..4 {
            for x in 0..4 {
                let pixel_x = xx * 4 + x;
                let pixel_y = yy * 4 + y;

                let offset = pixel_y * stride + pixel_x * 4;
                let alpha = rgba_data[offset + 3] as f32 / 255.0;

                // Convert alpha to 4 bits (0-15)
                let alpha4 = (alpha * 15.0) as u32;
                let bit_position = y * 16 + x * 4;

                if bit_position < 32 {
                    alpha_bits[0] |= alpha4 << bit_position;
                } else {
                    alpha_bits[1] |= alpha4 << (bit_position - 32);
                }
            }
        }

        alpha_bits
    }

    fn store_data(
        &self,
        blocks_buffer: &mut [u8],
        block_width: usize,
        xx: usize,
        yy: usize,
        data: &[u32],
    ) {
        let offset = (yy * block_width + xx) * (data.len() * 4);

        for (index, &value) in data.iter().enumerate() {
            let byte_offset = offset + index * 4;
            blocks_buffer[byte_offset] = value as u8;
            blocks_buffer[byte_offset + 1] = (value >> 8) as u8;
            blocks_buffer[byte_offset + 2] = (value >> 16) as u8;
            blocks_buffer[byte_offset + 3] = (value >> 24) as u8;
        }
    }

    fn compute_covar_dc(&self, covar: &mut [f32; 6], dc: &mut [f32; 3]) {
        for (p, value) in dc.iter_mut().enumerate() {
            let mut acc = 0.0;
            for k in 0..16 {
                acc += self.block[k + p * 16];
            }
            *value = acc / 16.0;
        }

        let mut covar0 = 0.0;
        let mut covar1 = 0.0;
        let mut covar2 = 0.0;
        let mut covar3 = 0.0;
        let mut covar4 = 0.0;
        let mut covar5 = 0.0;

        for k in 0..16 {
            let rgb0 = self.block[k] - dc[0];
            let rgb1 = self.block[k + 16] - dc[1];
            let rgb2 = self.block[k + 32] - dc[2];

            covar0 += rgb0 * rgb0;
            covar1 += rgb0 * rgb1;
            covar2 += rgb0 * rgb2;
            covar3 += rgb1 * rgb1;
            covar4 += rgb1 * rgb2;
            covar5 += rgb2 * rgb2;
        }

        covar[0] = covar0;
        covar[1] = covar1;
        covar[2] = covar2;
        covar[3] = covar3;
        covar[4] = covar4;
        covar[5] = covar5;
    }

    fn ssymv(&self, result: &mut [f32; 3], covar: &[f32; 6], a_vector: &[f32; 3]) {
        result[0] = covar[0] * a_vector[0] + covar[1] * a_vector[1] + covar[2] * a_vector[2];
        result[1] = covar[1] * a_vector[0] + covar[3] * a_vector[1] + covar[4] * a_vector[2];
        result[2] = covar[2] * a_vector[0] + covar[4] * a_vector[1] + covar[5] * a_vector[2];
    }

    fn compute_axis3(&self, axis: &mut [f32; 3], covar: &[f32; 6], power_iterations: i32) {
        let mut a_vector = [1.0; 3];

        for i in 0..power_iterations {
            self.ssymv(axis, covar, &a_vector);

            a_vector.copy_from_slice(&axis[..]);

            if i % 2 == 1 {
                let mut norm_sq = 0.0;
                for value in axis.iter() {
                    norm_sq += value * value;
                }

                let rnorm = 1.0 / norm_sq.sqrt();

                for value in a_vector.iter_mut() {
                    *value *= rnorm;
                }
            }
        }

        axis.copy_from_slice(&a_vector);
    }

    fn pick_endpoints(&self, c0: &mut [f32; 3], c1: &mut [f32; 3], axis: &[f32; 3], dc: &[f32; 3]) {
        let mut min_dot: f32 = 256.0 * 256.0;
        let mut max_dot: f32 = 0.0;

        for y in 0..4 {
            for x in 0..4 {
                let mut dot = 0.0;
                for p in 0..3 {
                    dot += (self.block[p * 16 + y * 4 + x] - dc[p]) * axis[p];
                }

                min_dot = f32::min(min_dot, dot);
                max_dot = f32::max(max_dot, dot);
            }
        }

        if max_dot - min_dot < 1.0 {
            min_dot -= 0.5;
            max_dot += 0.5;
        }

        let mut norm_sq = 0.0;
        for value in axis.iter() {
            norm_sq += *value * *value;
        }

        let rnorm_sq = norm_sq.recip();
        for p in 0..3 {
            c0[p] = f32::clamp(dc[p] + min_dot * rnorm_sq * axis[p], 0.0, 255.0);
            c1[p] = f32::clamp(dc[p] + max_dot * rnorm_sq * axis[p], 0.0, 255.0);
        }
    }

    fn dec_rgb565(&self, c: &mut [f32; 3], p: i32) {
        let b5 = p & 31;
        let g6 = (p >> 5) & 63;
        let r5 = (p >> 11) & 31;

        c[0] = ((r5 << 3) + (r5 >> 2)) as f32;
        c[1] = ((g6 << 2) + (g6 >> 4)) as f32;
        c[2] = ((b5 << 3) + (b5 >> 2)) as f32;
    }

    fn enc_rgb565(&self, c: &[f32; 3]) -> i32 {
        let r = c[0] as i32;
        let g = c[1] as i32;
        let b = c[2] as i32;

        let r5 = (r * 31 + 128 + ((r * 31) >> 8)) >> 8;
        let g6 = (g * 63 + 128 + ((g * 63) >> 8)) >> 8;
        let b5 = (b * 31 + 128 + ((b * 31) >> 8)) >> 8;

        (r5 << 11) + (g6 << 5) + b5
    }

    fn fast_quant(&self, p0: i32, p1: i32) -> u32 {
        let mut c0 = [0.0; 3];
        let mut c1 = [0.0; 3];
        self.dec_rgb565(&mut c0, p0);
        self.dec_rgb565(&mut c1, p1);

        let mut dir = [0.0; 3];
        for p in 0..3 {
            dir[p] = c1[p] - c0[p];
        }

        let mut sq_norm = 0.0;
        for value in dir.iter() {
            sq_norm += value.powi(2);
        }

        let rsq_norm = sq_norm.recip();

        for value in dir.iter_mut() {
            *value *= rsq_norm * 3.0;
        }

        let mut bias = 0.5;
        for p in 0..3 {
            bias -= c0[p] * dir[p];
        }

        let mut bits = 0;
        let mut scaler = 1;
        for k in 0..16 {
            let mut dot = 0.0;
            for (p, value) in dir.iter().enumerate() {
                dot += self.block[k + p * 16] * value;
            }

            let q = i32::clamp((dot + bias) as i32, 0, 3);
            bits += q as u32 * scaler;
            scaler = scaler.wrapping_mul(4);
        }

        bits
    }

    fn bc1_refine(&self, pe: &mut [i32; 2], bits: u32, dc: &[f32; 3]) {
        let mut c0 = [0.0; 3];
        let mut c1 = [0.0; 3];

        if (bits ^ (bits.wrapping_mul(4))) < 4 {
            c0.copy_from_slice(&dc[..]);
            c1.copy_from_slice(&dc[..]);
        } else {
            let mut atb1 = [0.0; 3];
            let mut sum_q = 0.0;
            let mut sum_qq = 0.0;
            let mut shifted_bits = bits;

            for k in 0..16 {
                let q = (shifted_bits & 3) as f32;
                shifted_bits >>= 2;

                let x = 3.0 - q;

                sum_q += q;
                sum_qq += q * q;

                for (p, value) in atb1.iter_mut().enumerate() {
                    *value += x * self.block[k + p * 16];
                }
            }

            let mut sum = [0.0; 3];
            let mut atb2 = [0.0; 3];

            for p in 0..3 {
                sum[p] = dc[p] * 16.0;
                atb2[p] = 3.0 * sum[p] - atb1[p];
            }

            let cxx = 16.0 * 9.0 - 2.0 * 3.0 * sum_q + sum_qq;
            let cyy = sum_qq;
            let cxy = 3.0 * sum_q - sum_qq;
            let scale = 3.0 * (cxx * cyy - cxy * cxy).recip();

            for p in 0..3 {
                c0[p] = (atb1[p] * cyy - atb2[p] * cxy) * scale;
                c1[p] = (atb2[p] * cxx - atb1[p] * cxy) * scale;

                c0[p] = f32::clamp(c0[p], 0.0, 255.0);
                c1[p] = f32::clamp(c1[p], 0.0, 255.0);
            }
        }

        pe[0] = self.enc_rgb565(&c0);
        pe[1] = self.enc_rgb565(&c1);
    }

    fn fix_qbits(&self, qbits: u32) -> u32 {
        const MASK_01B: u32 = 0x55555555;
        const MASK_10B: u32 = 0xAAAAAAAA;

        let qbits0 = qbits & MASK_01B;
        let qbits1 = qbits & MASK_10B;

        (qbits1 >> 1) + (qbits1 ^ (qbits0 << 1))
    }

    fn compress_block_bc1_core(&self) -> [u32; 2] {
        let power_iterations = 4;
        let refine_iterations = 1;

        let mut covar = [0.0; 6];
        let mut dc = [0.0; 3];
        self.compute_covar_dc(&mut covar, &mut dc);

        const EPS: f32 = 0.001;
        covar[0] += EPS;
        covar[3] += EPS;
        covar[5] += EPS;

        let mut axis = [0.0; 3];
        self.compute_axis3(&mut axis, &covar, power_iterations);

        let mut c0 = [0.0; 3];
        let mut c1 = [0.0; 3];
        self.pick_endpoints(&mut c0, &mut c1, &axis, &dc);

        let mut p = [0; 2];
        p[0] = self.enc_rgb565(&c0);
        p[1] = self.enc_rgb565(&c1);
        if p[0] < p[1] {
            p.swap(0, 1);
        }

        let mut data = [0; 2];
        data[0] = ((p[1] as u32) << 16) | p[0] as u32;
        data[1] = self.fast_quant(p[0], p[1]);

        for _ in 0..refine_iterations {
            self.bc1_refine(&mut p, data[1], &dc);
            if p[0] < p[1] {
                p.swap(0, 1);
            }
            data[0] = ((p[1] as u32) << 16) | p[0] as u32;
            data[1] = self.fast_quant(p[0], p[1]);
        }

        data[1] = self.fix_qbits(data[1]);

        data
    }

    fn compress_block_bc3_alpha(&self) -> [u32; 2] {
        let mut ep = [255.0, 0.0];

        // Find min/max endpoints using block[48] to block[63] for alpha
        for k in 0..16 {
            ep[0] = f32::min(ep[0], self.block[48 + k]);
            ep[1] = f32::max(ep[1], self.block[48 + k]);
        }

        // Prevent division by zero
        if ep[0] == ep[1] {
            ep[1] = ep[0] + 0.1;
        }

        let mut qblock = [0; 2];
        let scale = 7.0 / (ep[1] - ep[0]);

        for k in 0..16 {
            let v = self.block[48 + k];
            let proj = (v - ep[0]) * scale + 0.5;

            let mut q = i32::clamp(proj as i32, 0, 7);
            q = 7 - q;

            if q > 0 {
                q += 1;
            }
            if q == 8 {
                q = 1;
            }

            qblock[k / 8] |= (q as u32) << ((k % 8) * 3);
        }

        let mut data = [0; 2];
        data[0] = (u32::clamp(ep[0] as u32, 0, 255) << 8) | u32::clamp(ep[1] as u32, 0, 255);
        data[0] |= qblock[0] << 16;
        data[1] = qblock[0] >> 16;
        data[1] |= qblock[1] << 8;

        data
    }
}

/// Compresses raw RGBA8 data into BC1-5 block compressed format.
///
/// This function provides CPU-based texture compression for RGBA8 data. It supports BC1 through BC5
/// compression formats, with BC6H and BC7 being currently unimplemented.
///
/// # Data Layout Requirements
/// The input data must be in RGBA8 format (8 bits per channel, 32 bits per pixel). The data is
/// expected to be in row-major order, with optional stride for padding between rows.
///
/// # Buffer Requirements
/// The destination buffer must have sufficient capacity to store the compressed blocks.
/// The required size can be calculated using [`CompressionVariant::blocks_byte_size()`].
///
/// For example:
/// ```ignore
/// let required_size = variant.blocks_byte_size(width, height);
/// assert!(blocks_buffer.len() &gt;= required_size);
/// ```
///
/// # Arguments
/// * `variation` - The block compression format to use
/// * `rgba_data` - Source RGBA8 pixel data
/// * `blocks_buffer` - Destination buffer for the compressed blocks
/// * `width` - Width of the image in pixels
/// * `height` - Height of the image in pixels
/// * `stride` - Number of bytes per row in the source data (for padding).
///              Must be `width * 4` for tightly packed RGBA data.
///
/// # Panics
/// * If `width` or `height` is not a multiple of 4
/// * If the destination `blocks_buffer` is too small to hold the compressed data
/// * If BC6H or BC7 compression is requested (currently unimplemented)
///
/// # Example
/// ```
/// use block_compression::{encode::compress_rgba8, CompressionVariant};
///
/// let rgba_data = vec![0u8; 256 * 256 * 4]; // Your RGBA data
/// let width = 256;
/// let height = 256;
/// let stride = width * 4; // Tightly packed rows
/// let variant = CompressionVariant::BC1;
///
/// let mut blocks_buffer = vec![0u8; variant.blocks_byte_size(width, height)];
///
/// compress_rgba8(
///     variant,
///     &rgba_data,
///     &mut blocks_buffer,
///     width,
///     height,
///     stride,
/// );
/// ```
pub fn compress_rgba8(
    variation: CompressionVariant,
    rgba_data: &[u8],
    blocks_buffer: &mut [u8],
    width: u32,
    height: u32,
    stride: u32,
) {
    assert_eq!(height % 4, 0);
    assert_eq!(width % 4, 0);

    let required_size = CompressionVariant::BC1.blocks_byte_size(width, height);

    assert!(
        blocks_buffer.len() >= required_size,
        "blocks_buffer size ({}) is too small to hold compressed blocks. Required size: {}",
        blocks_buffer.len(),
        required_size
    );

    let stride = stride as usize;
    let block_width = (width as usize + 3) / 4;
    let block_height = (height as usize + 3) / 4;

    match variation {
        CompressionVariant::BC1 => {
            compress_bc1(rgba_data, blocks_buffer, block_width, block_height, stride);
        }
        CompressionVariant::BC2 => {
            compress_bc2(rgba_data, blocks_buffer, block_width, block_height, stride);
        }
        CompressionVariant::BC3 => {
            compress_bc3(rgba_data, blocks_buffer, block_width, block_height, stride);
        }
        CompressionVariant::BC4 => {
            compress_bc4(rgba_data, blocks_buffer, block_width, block_height, stride);
        }
        CompressionVariant::BC5 => {
            compress_bc5(rgba_data, blocks_buffer, block_width, block_height, stride);
        }
        #[cfg(feature = "bc6h")]
        CompressionVariant::BC6H(_) => {
            unimplemented!("CPU based BC6H compression not yet implemented yet");
        }
        #[cfg(feature = "bc7")]
        CompressionVariant::BC7(_) => {
            unimplemented!("CPU based BC7 compression not yet implemented yet");
        }
    }
}

fn compress_bc1(
    rgba_data: &[u8],
    blocks_buffer: &mut [u8],
    block_width: usize,
    block_height: usize,
    stride: usize,
) {
    for yy in 0..block_height {
        for xx in 0..block_width {
            let mut block_compressor = BlockCompressorBC15::default();

            block_compressor.load_block_interleaved_rgba(rgba_data, xx, yy, stride);
            let color_result = block_compressor.compress_block_bc1_core();
            block_compressor.store_data(blocks_buffer, block_width, xx, yy, &color_result);
        }
    }
}

fn compress_bc2(
    rgba_data: &[u8],
    blocks_buffer: &mut [u8],
    block_width: usize,
    block_height: usize,
    stride: usize,
) {
    for yy in 0..block_height {
        for xx in 0..block_width {
            let mut block_compressor = BlockCompressorBC15::default();
            let mut compressed_data = [0; 4];

            let alpha_result = block_compressor.load_block_alpha_4bit(rgba_data, xx, yy, stride);

            compressed_data[0] = alpha_result[0];
            compressed_data[1] = alpha_result[1];

            block_compressor.load_block_interleaved_rgba(rgba_data, xx, yy, stride);

            let color_result = block_compressor.compress_block_bc1_core();
            compressed_data[2] = color_result[0];
            compressed_data[3] = color_result[1];

            block_compressor.store_data(blocks_buffer, block_width, xx, yy, &compressed_data);
        }
    }
}

fn compress_bc3(
    rgba_data: &[u8],
    blocks_buffer: &mut [u8],
    block_width: usize,
    block_height: usize,
    stride: usize,
) {
    for yy in 0..block_height {
        for xx in 0..block_width {
            let mut block_compressor = BlockCompressorBC15::default();

            let mut compressed_data = [0; 4];

            block_compressor.load_block_interleaved_rgba(rgba_data, xx, yy, stride);

            let alpha_result = block_compressor.compress_block_bc3_alpha();
            compressed_data[0] = alpha_result[0];
            compressed_data[1] = alpha_result[1];

            let color_result = block_compressor.compress_block_bc1_core();
            compressed_data[2] = color_result[0];
            compressed_data[3] = color_result[1];

            block_compressor.store_data(blocks_buffer, block_width, xx, yy, &compressed_data);
        }
    }
}

fn compress_bc4(
    rgba_data: &[u8],
    blocks_buffer: &mut [u8],
    block_width: usize,
    block_height: usize,
    stride: usize,
) {
    for yy in 0..block_height {
        for xx in 0..block_width {
            let mut block_compressor = BlockCompressorBC15::default();

            let mut compressed_data = [0; 2];

            block_compressor.load_block_r_8bit(rgba_data, xx, yy, stride);

            let color_result = block_compressor.compress_block_bc3_alpha();
            compressed_data[0] = color_result[0];
            compressed_data[1] = color_result[1];

            block_compressor.store_data(blocks_buffer, block_width, xx, yy, &compressed_data);
        }
    }
}

fn compress_bc5(
    rgba_data: &[u8],
    blocks_buffer: &mut [u8],
    block_width: usize,
    block_height: usize,
    stride: usize,
) {
    for yy in 0..block_height {
        for xx in 0..block_width {
            let mut block_compressor = BlockCompressorBC15::default();

            let mut compressed_data = [0; 4];

            block_compressor.load_block_r_8bit(rgba_data, xx, yy, stride);

            let red_result = block_compressor.compress_block_bc3_alpha();
            compressed_data[0] = red_result[0];
            compressed_data[1] = red_result[1];

            block_compressor.load_block_g_8bit(rgba_data, xx, yy, stride);

            let green_result = block_compressor.compress_block_bc3_alpha();
            compressed_data[2] = green_result[0];
            compressed_data[3] = green_result[1];

            block_compressor.store_data(blocks_buffer, block_width, xx, yy, &compressed_data);
        }
    }
}
