//! # block_compression
//!
//! Texture block compression using WGPU compute shader.
//! The shaders are a port of Intel's ISPC Texture Compressor's kernel to WGSL compute shader.
//!
//! ## Supported block compressions
//!
//! Currently supported block compressions are:
//!
//!  * BC1
//!  * BC2
//!  * BC3
//!  * BC4
//!  * BC5
//!
//! Soon:
//!
//!  * BC6H
//!  * BC7
mod block_compressor;
mod settings;

pub use block_compressor::BlockCompressor;
pub use settings::{BC6HSettings, BC7Settings};

// TODO: NHA Implement BC6H
// TODO: NHA Implement BC7

/// Compression variants supported by this crate.
#[derive(Copy, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Debug)]
pub enum CompressionVariant {
    /// BC1 compression (RGB)
    BC1,
    /// BC2 compression (RGBA)
    BC2,
    /// BC3 compression (RGBA)
    BC3,
    /// BC4 compression (R)
    BC4,
    /// BC5 compression (RG)
    BC5,
    /// BC6H compression (RGB HDR)
    BC6H,
    /// BC7 compression (RGBA)
    BC7,
}

impl CompressionVariant {
    /// Returns the byte size required for storing compressed blocks for the given dimensions.
    ///
    /// The size is calculated based on the block compression format and rounded up dimensions.
    /// Width and height are rounded up to the nearest multiple of 4.
    pub fn blocks_byte_size(self, width: u32, height: u32) -> usize {
        let block_width = (width as usize + 3) / 4;
        let block_height = (height as usize + 3) / 4;
        let block_count = block_width * block_height;
        let block_size = self.block_byte_size() as usize;
        block_count * block_size
    }

    fn block_byte_size(self) -> u32 {
        match self {
            CompressionVariant::BC1 | CompressionVariant::BC4 => 8,
            CompressionVariant::BC2
            | CompressionVariant::BC3
            | CompressionVariant::BC5
            | CompressionVariant::BC6H
            | CompressionVariant::BC7 => 16,
        }
    }

    fn name(self) -> &'static str {
        match self {
            CompressionVariant::BC1 => "bc1",
            CompressionVariant::BC2 => "bc2",
            CompressionVariant::BC3 => "bc3",
            CompressionVariant::BC4 => "bc4",
            CompressionVariant::BC5 => "bc5",
            CompressionVariant::BC6H => "bc6h",
            CompressionVariant::BC7 => "bc7",
        }
    }

    fn entry_point(self) -> &'static str {
        match self {
            CompressionVariant::BC1 => "compress_bc1",
            CompressionVariant::BC2 => "compress_bc2",
            CompressionVariant::BC3 => "compress_bc3",
            CompressionVariant::BC4 => "compress_bc4",
            CompressionVariant::BC5 => "compress_bc5",
            CompressionVariant::BC6H => "compress_bc6h",
            CompressionVariant::BC7 => "compress_bc7",
        }
    }
}
