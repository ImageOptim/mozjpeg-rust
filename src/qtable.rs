#![allow(non_upper_case_globals)]

use std::cmp::{max, min};
use std::fmt;
use std::os::raw::c_uint;
type Coef = c_uint;

pub struct QTable {
    pub(crate) coeffs: [Coef; 64],
}

impl PartialEq for QTable {
    fn eq(&self, other: &Self) -> bool {
        let iter2 = other.coeffs.iter().copied();
        self.coeffs.iter().copied().zip(iter2).all(|(s, o)| s == o)
    }
}

impl fmt::Debug for QTable {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(fmt, "QTable{{coeffs:{:?}}}", &self.coeffs[..])
    }
}

const low_weights : [f32; 19] = [
    1.00, 0.85, 0.55, 0., 0., 0., 0., 0.,
    0.85, 0.75, 0.10, 0., 0., 0., 0., 0.,
    0.55, 0.10, 0.05,
];

impl QTable {
    #[must_use]
    pub fn compare(&self, other: &Self) -> (f32, f32) {
        let mut scales = [0.; 64];
        for (s, (&a, &b)) in scales.iter_mut().zip(self.coeffs.iter().zip(other.coeffs.iter())) {
            *s = if b > 0 {a as f32 / b as f32} else {0.};
        }
        let avg = scales.iter().sum::<f32>() / 64.;
        let var = scales.iter().map(|&v| (v - avg).powi(2)).sum::<f32>() / 64.;
        (avg, var)
    }

    #[must_use]
    pub fn scaled(&self, dc_quality: f32, ac_quality: f32) -> Self {
        let dc_scaling = Self::quality_scaling(dc_quality);
        let ac_scaling = Self::quality_scaling(ac_quality);

        let mut out = [0; 64];
        {
            debug_assert_eq!(self.coeffs.len(), out.len());
            debug_assert!(low_weights.len() < self.coeffs.len());

            let (low_coefs, high_coefs) = self.coeffs.split_at(low_weights.len());
            let (low_out, high_out) = out.split_at_mut(low_weights.len());

            // TODO: that could be improved for 1x2 and 2x1 subsampling
            for ((out, coef), w) in low_out.iter_mut().zip(low_coefs).zip(&low_weights) {
                *out = min(255, max(1, (*coef as f32 * (dc_scaling * w + ac_scaling * (1.-w))).round() as Coef));
            }
            for (out, coef) in high_out.iter_mut().zip(high_coefs) {
                *out = min(255, max(1, (*coef as f32 * ac_scaling).round() as Coef));
            }
        }
        Self { coeffs: out }
    }

    #[must_use]
    pub fn as_ptr(&self) -> *const c_uint {
        self.coeffs.as_ptr()
    }

    // Similar to libjpeg, but result is 100x smaller
    fn quality_scaling(quality: f32) -> f32 {
        assert!(quality > 0. && quality <= 100.);

        if quality < 50. {
            50. / quality
        } else {
            (100. - quality) / 50.
        }
    }
}

pub static AnnexK_Luma: QTable = QTable {
    coeffs: [
        16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69,
        56, 14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81,
        104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
    ],
};

pub static AnnexK_Chroma: QTable = QTable {
    coeffs: [
        17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99,
        99, 47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
        99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    ],
};

pub static Flat: QTable = QTable {
    coeffs: [
        16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
        16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
        16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    ],
};

pub static MSSSIM_Luma: QTable = QTable {
    coeffs: [
        12, 17, 20, 21, 30, 34, 56, 63, 18, 20, 20, 26, 28, 51, 61, 55, 19, 20, 21, 26, 33, 58, 69,
        55, 26, 26, 26, 30, 46, 87, 86, 66, 31, 33, 36, 40, 46, 96, 100, 73, 40, 35, 46, 62, 81,
        100, 111, 91, 46, 66, 76, 86, 102, 121, 120, 101, 68, 90, 90, 96, 113, 102, 105, 103,
    ],
};

pub static MSSSIM_Chroma: QTable = QTable {
    coeffs: [
        8, 12, 15, 15, 86, 96, 96, 98, 13, 13, 15, 26, 90, 96, 99, 98, 12, 15, 18, 96, 99, 99, 99,
        99, 17, 16, 90, 96, 99, 99, 99, 99, 96, 96, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
        99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    ],
};

pub static NRobidoux: QTable = QTable {
    coeffs: [
        16, 16, 16, 18, 25, 37, 56, 85, 16, 17, 20, 27, 34, 40, 53, 75, 16, 20, 24, 31, 43, 62, 91,
        135, 18, 27, 31, 40, 53, 74, 106, 156, 25, 34, 43, 53, 69, 94, 131, 189, 37, 40, 62, 74,
        94, 124, 169, 238, 56, 53, 91, 106, 131, 169, 226, 311, 85, 75, 135, 156, 189, 238, 311,
        418,
    ],
};

pub static PSNRHVS_Luma: QTable = QTable {
    coeffs: [
        9, 10, 12, 14, 27, 32, 51, 62, 11, 12, 14, 19, 27, 44, 59, 73, 12, 14, 18, 25, 42, 59, 79,
        78, 17, 18, 25, 42, 61, 92, 87, 92, 23, 28, 42, 75, 79, 112, 112, 99, 40, 42, 59, 84, 88,
        124, 132, 111, 42, 64, 78, 95, 105, 126, 125, 99, 70, 75, 100, 102, 116, 100, 107, 98,
    ],
};
pub static PSNRHVS_Chroma: QTable = QTable {
    coeffs: [
        9, 10, 17, 19, 62, 89, 91, 97, 12, 13, 18, 29, 84, 91, 88, 98, 14, 19, 29, 93, 95, 95, 98,
        97, 20, 26, 84, 88, 95, 95, 98, 94, 26, 86, 91, 93, 97, 99, 98, 99, 99, 100, 98, 99, 99,
        99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 97, 97, 99, 99, 99, 99, 97, 99,
    ],
};

pub static KleinSilversteinCarney: QTable = QTable {
    coeffs: [
        /* Relevance of human vision to JPEG-DCT compression (1992) Klein, Silverstein and Carney.
         */
        10, 12, 14, 19, 26, 38, 57, 86, 12, 18, 21, 28, 35, 41, 54, 76, 14, 21, 25, 32, 44, 63, 92,
        136, 19, 28, 32, 41, 54, 75, 107, 157, 26, 35, 44, 54, 70, 95, 132, 190, 38, 41, 63, 75,
        95, 125, 170, 239, 57, 54, 92, 107, 132, 170, 227, 312, 86, 76, 136, 157, 190, 239, 312,
        419,
    ],
};

pub static WatsonTaylorBorthwick: QTable = QTable {
    coeffs: [
        /* DCTune perceptual optimization of compressed dental X-Rays (1997) Watson, Taylor, Borthwick
         */
        7, 8, 10, 14, 23, 44, 95, 241, 8, 8, 11, 15, 25, 47, 102, 255, 10, 11, 13, 19, 31, 58, 127,
        255, 14, 15, 19, 27, 44, 83, 181, 255, 23, 25, 31, 44, 72, 136, 255, 255, 44, 47, 58, 83,
        136, 255, 255, 255, 95, 102, 127, 181, 255, 255, 255, 255, 241, 255, 255, 255, 255, 255,
        255, 255,
    ],
};

pub static AhumadaWatsonPeterson: QTable = QTable {
    coeffs: [
        /* A visual detection model for DCT coefficient quantization (12/9/93) Ahumada, Watson, Peterson
         */
        15, 11, 11, 12, 15, 19, 25, 32, 11, 13, 10, 10, 12, 15, 19, 24, 11, 10, 14, 14, 16, 18, 22,
        27, 12, 10, 14, 18, 21, 24, 28, 33, 15, 12, 16, 21, 26, 31, 36, 42, 19, 15, 18, 24, 31, 38,
        45, 53, 25, 19, 22, 28, 36, 45, 55, 65, 32, 24, 27, 33, 42, 53, 65, 77,
    ],
};

pub static PetersonAhumadaWatson: QTable = QTable {
    coeffs: [
        /* An improved detection model for DCT coefficient quantization (1993) Peterson, Ahumada and Watson
         */
        14, 10, 11, 14, 19, 25, 34, 45, 10, 11, 11, 12, 15, 20, 26, 33, 11, 11, 15, 18, 21, 25, 31,
        38, 14, 12, 18, 24, 28, 33, 39, 47, 19, 15, 21, 28, 36, 43, 51, 59, 25, 20, 25, 33, 43, 54,
        64, 74, 34, 26, 31, 39, 51, 64, 77, 91, 45, 33, 38, 47, 59, 74, 91, 108,
    ],
};

pub static ALL_TABLES: [(&str, &QTable); 12] = [
    ("Annex-K Luma", &AnnexK_Luma),
    ("Annex-K Chroma", &AnnexK_Chroma),
    ("Flat", &Flat),
    ("MSSSIM Luma", &MSSSIM_Luma),
    ("MSSSIM Chroma", &MSSSIM_Chroma),
    ("N. Robidoux", &NRobidoux),
    ("PSNRHVS Luma", &PSNRHVS_Luma),
    ("PSNRHVS Chroma", &PSNRHVS_Chroma),
    ("Klein, Silverstein, Carney", &KleinSilversteinCarney),
    ("Watson, Taylor, Borthwick", &WatsonTaylorBorthwick),
    ("Ahumada, Watson, Peterson", &AhumadaWatsonPeterson),
    ("Peterson, Ahumada, Watson", &PetersonAhumadaWatson),
];

#[test]
fn scaling() {
    assert_eq!(QTable { coeffs: [100; 64] }, QTable { coeffs: [100; 64] });
    assert!(QTable { coeffs: [1; 64] } != QTable { coeffs: [2; 64] });

    assert_eq!(QTable{coeffs:[36; 64]}, Flat.scaled(22.,22.));
    assert_eq!(QTable{coeffs:[8; 64]}, Flat.scaled(75.,75.));
    assert_eq!(QTable{coeffs:[1; 64]}, Flat.scaled(100.,100.));
    assert_eq!(QTable{coeffs:[2; 64]}, Flat.scaled(95.,95.));
    assert_eq!(QTable{coeffs:[
         2,  6, 15, 32, 32, 32, 32, 32,
         6,  9, 29, 32, 32, 32, 32, 32,
        15, 29, 30, 32, 32, 32, 32, 32,
        32, 32, 32, 32, 32, 32, 32, 32,
        32, 32, 32, 32, 32, 32, 32, 32,
        32, 32, 32, 32, 32, 32, 32, 32,
        32, 32, 32, 32, 32, 32, 32, 32,
        32, 32, 32, 32, 32, 32, 32, 32]}, Flat.scaled(95.,25.));
    assert_eq!(PetersonAhumadaWatson, PetersonAhumadaWatson.scaled(50.,50.));

    assert_eq!(QTable { coeffs: [1; 64] }, NRobidoux.scaled(99.9, 99.9));
    assert_eq!(QTable { coeffs: [1; 64] }, MSSSIM_Chroma.scaled(99.8, 99.8));
}
