#[derive(Debug, Clone)]
pub struct PsnrResult {
    pub overall_psnr: f64,
    pub overall_mse: f64,
    pub channel_results: ChannelResults,
}

#[derive(Debug, Clone)]
pub struct ChannelResults {
    pub red: ChannelMetrics,
    pub green: ChannelMetrics,
    pub blue: ChannelMetrics,
    pub alpha: ChannelMetrics,
}

#[derive(Debug, Clone)]
pub struct ChannelMetrics {
    pub psnr: f64,
    pub mse: f64,
}

/// Calculates quality metrics for a given image. The input data and output data must be RGBA data.
pub fn calculate_image_metrics(
    original: &[u8],
    compressed: &[u8],
    width: u32,
    height: u32,
    channels: u32,
) -> PsnrResult {
    if original.len() != compressed.len() {
        panic!("Image buffers must have same length");
    }
    if original.len() != (width * height * 4) as usize {
        panic!("Buffer size doesn't match dimensions");
    }

    let mut channel_mse = [0.0; 4];
    let pixel_count = (width * height) as f64;

    for index in (0..original.len()).step_by(4) {
        for channel in 0..4 {
            let orig = if channel < 3 {
                srgb_to_linear(original[index + channel])
            } else {
                (original[index + channel] as f64) / 255.0
            };

            let comp = if channel < 3 {
                srgb_to_linear(compressed[index + channel])
            } else {
                (compressed[index + channel] as f64) / 255.0
            };

            let diff = orig - comp;
            channel_mse[channel] += diff * diff;
        }
    }

    // Normalize MSE values
    channel_mse.iter_mut().for_each(|mse| *mse /= pixel_count);

    let calculate_psnr = |mse: f64| -> f64 {
        if mse == 0.0 {
            0.0
        } else {
            20.0 * (1.0 / mse.sqrt()).log10()
        }
    };

    let overall_mse = channel_mse.iter().sum::<f64>() / channels as f64;
    let overall_psnr = calculate_psnr(overall_mse);

    let channel_results = ChannelResults {
        red: ChannelMetrics {
            mse: channel_mse[0],
            psnr: calculate_psnr(channel_mse[0]),
        },
        green: ChannelMetrics {
            mse: channel_mse[1],
            psnr: calculate_psnr(channel_mse[1]),
        },
        blue: ChannelMetrics {
            mse: channel_mse[2],
            psnr: calculate_psnr(channel_mse[2]),
        },
        alpha: ChannelMetrics {
            mse: channel_mse[3],
            psnr: calculate_psnr(channel_mse[3]),
        },
    };

    PsnrResult {
        overall_psnr,
        overall_mse,
        channel_results,
    }
}

#[inline]
fn srgb_to_linear(srgb: u8) -> f64 {
    let v = (srgb as f64) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
