pub use crate::ffi::jpeg_component_info as CompInfo;
use crate::ffi::DCTSIZE;
use crate::qtable::QTable;

pub trait CompInfoExt {
    /// Number of pixels per row, including padding to MCU
    fn row_stride(&self) -> usize;
    /// Total height, including padding to MCU
    fn col_stride(&self) -> usize;

    /// h,v samplinig (1..4). Number of pixels per sample (may be opposite of what you expect!)
    fn sampling(&self) -> (u8, u8);

    // Quantization table, if available
    fn qtable(&self) -> Option<QTable>;

    // Number of blocks per row
    fn width_in_blocks(&self) -> usize;

    // Number of block rows
    fn height_in_blocks(&self) -> usize;
}

impl CompInfoExt for CompInfo {
    fn qtable(&self) -> Option<QTable> {
        unsafe { self.quant_table.as_ref() }.map(|q_in| {
            let mut qtable = QTable { coeffs: [0; 64] };
            for (out, q) in qtable.coeffs.iter_mut().zip(q_in.quantval.iter()) {
                *out = u32::from(*q);
            }
            qtable
        })
    }

    fn sampling(&self) -> (u8, u8) {
        (self.h_samp_factor as u8, self.v_samp_factor as u8)
    }

    fn row_stride(&self) -> usize {
        assert!(self.width_in_blocks > 0);
        self.width_in_blocks as usize * DCTSIZE
    }

    fn col_stride(&self) -> usize {
        assert!(self.height_in_blocks > 0);
        self.height_in_blocks as usize * DCTSIZE
    }

    fn width_in_blocks(&self) -> usize {
        self.width_in_blocks as _
    }

    fn height_in_blocks(&self) -> usize {
        self.height_in_blocks as _
    }
}
