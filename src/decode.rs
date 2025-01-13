mod block;

use self::block::{
    decode_block_bc1, decode_block_bc2, decode_block_bc3, decode_block_bc4, decode_block_bc5,
};
use crate::CompressionVariant;

trait BlockDecoder {
    fn decode_block(compressed: &[u8], decompressed: &mut [u8], pitch: usize);
    fn block_byte_size() -> u32;
    fn bytes_per_pixel() -> u32;
}

struct BC1Decoder;
struct BC2Decoder;
struct BC3Decoder;
struct BC4Decoder;
struct BC5Decoder;

impl BlockDecoder for BC1Decoder {
    #[inline(always)]
    fn decode_block(compressed: &[u8], decompressed: &mut [u8], pitch: usize) {
        decode_block_bc1(compressed, decompressed, pitch)
    }

    fn block_byte_size() -> u32 {
        CompressionVariant::BC1.block_byte_size()
    }

    fn bytes_per_pixel() -> u32 {
        CompressionVariant::BC1.bytes_per_pixel()
    }
}

impl BlockDecoder for BC2Decoder {
    #[inline(always)]
    fn decode_block(compressed: &[u8], decompressed: &mut [u8], pitch: usize) {
        decode_block_bc2(compressed, decompressed, pitch)
    }

    fn block_byte_size() -> u32 {
        CompressionVariant::BC2.block_byte_size()
    }

    fn bytes_per_pixel() -> u32 {
        CompressionVariant::BC2.bytes_per_pixel()
    }
}

impl BlockDecoder for BC3Decoder {
    #[inline(always)]
    fn decode_block(compressed: &[u8], decompressed: &mut [u8], pitch: usize) {
        decode_block_bc3(compressed, decompressed, pitch)
    }

    fn block_byte_size() -> u32 {
        CompressionVariant::BC3.block_byte_size()
    }

    fn bytes_per_pixel() -> u32 {
        CompressionVariant::BC3.bytes_per_pixel()
    }
}

impl BlockDecoder for BC4Decoder {
    #[inline(always)]
    fn decode_block(compressed: &[u8], decompressed: &mut [u8], pitch: usize) {
        decode_block_bc4(compressed, decompressed, pitch)
    }

    fn block_byte_size() -> u32 {
        CompressionVariant::BC4.block_byte_size()
    }

    fn bytes_per_pixel() -> u32 {
        CompressionVariant::BC4.bytes_per_pixel()
    }
}

impl BlockDecoder for BC5Decoder {
    #[inline(always)]
    fn decode_block(compressed: &[u8], decompressed: &mut [u8], pitch: usize) {
        decode_block_bc5(compressed, decompressed, pitch)
    }

    fn block_byte_size() -> u32 {
        CompressionVariant::BC5.block_byte_size()
    }

    fn bytes_per_pixel() -> u32 {
        CompressionVariant::BC5.bytes_per_pixel()
    }
}

fn decompress<D: BlockDecoder>(
    width: u32,
    height: u32,
    input_bitstream: &[u8],
    output_bitstream: &mut [u8],
) {
    let blocks_x = (width + 3) / 4;
    let blocks_y = (height + 3) / 4;
    let block_byte_size = D::block_byte_size() as usize;
    let row_pitch = width as usize * D::bytes_per_pixel() as usize;

    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let block_index = (by * blocks_x + bx) as usize;
            let block_offset = block_index * block_byte_size;

            if block_offset + block_byte_size > input_bitstream.len() {
                break;
            }

            let output_offset = (by * 4 * row_pitch as u32 + bx * 16) as usize;

            if output_offset < output_bitstream.len() {
                D::decode_block(
                    &input_bitstream[block_offset..block_offset + block_byte_size],
                    &mut output_bitstream[output_offset..],
                    row_pitch,
                );
            }
        }
    }
}

pub fn decompress_blocks(
    variant: CompressionVariant,
    width: u32,
    height: u32,
    input_bitstream: &[u8],
    output_bitstream: &mut [u8],
) {
    let expected_input_size = variant.blocks_byte_size(width, height);
    if input_bitstream.len() != expected_input_size {
        panic!("the input bitstream slice has not the expected size");
    }

    let expected_output_size = width as usize * height as usize * 4;
    if output_bitstream.len() != expected_output_size {
        panic!("the output slice has not the expected size");
    }

    match variant {
        CompressionVariant::BC1 => {
            decompress::<BC1Decoder>(width, height, input_bitstream, output_bitstream)
        }
        CompressionVariant::BC2 => {
            decompress::<BC2Decoder>(width, height, input_bitstream, output_bitstream)
        }
        CompressionVariant::BC3 => {
            decompress::<BC3Decoder>(width, height, input_bitstream, output_bitstream)
        }
        CompressionVariant::BC4 => {
            decompress::<BC4Decoder>(width, height, input_bitstream, output_bitstream)
        }
        CompressionVariant::BC5 => {
            decompress::<BC5Decoder>(width, height, input_bitstream, output_bitstream)
        }
        _ => panic!("Unsupported compression variant"),
    }
}
