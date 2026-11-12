#![allow(
    clippy::excessive_precision,
    reason = "OK color conversion constants retain their published precision"
)]
use crate::model::oklab::OkLab;

/// Reference chroma milestones (initial slope, midpoint, and gamut maximum) for a given [`OkLab`] lightness and hue.
#[derive(Clone, Copy)]
pub struct ChromaValues {
    pub zero: f32,
    pub mid: f32,
    pub max: f32,
}

impl ChromaValues {
    pub fn from_normalized(lightness: f32, a_: f32, b_: f32) -> Self {
        let cusp = find_cusp(a_, b_);
        let max_chroma = find_gamut_intersection(a_, b_, lightness, 1.0, lightness, cusp);
        let st_max = ST::from_lc(cusp);
        let k = max_chroma / (lightness * st_max.s).min((1.0 - lightness) * st_max.t);
        let st_mid = ST::mid(a_, b_);
        let c_a = lightness * st_mid.s;
        let c_b = (1.0 - lightness) * st_mid.t;
        let c_mid = 0.9
            * k
            * (1.0 / (1.0 / c_a.powi(4) + 1.0 / c_b.powi(4)))
                .sqrt()
                .sqrt();
        let c_a = lightness * 0.4;
        let c_b = (1.0 - lightness) * 0.8;
        let c_0 = (1.0 / (1.0 / c_a.powi(2) + 1.0 / c_b.powi(2))).sqrt();

        Self {
            zero: c_0,
            mid: c_mid,
            max: max_chroma,
        }
    }
}

/// Lightness and chroma coordinates of a gamut cusp in [`OkLab`] space.
#[derive(Clone, Copy)]
pub struct LC {
    lightness: f32,
    chroma: f32,
}

/// Triangle slope parameters (`s` and `t`) describing the sRGB gamut boundary in [`OkLab`] space.
#[derive(Clone, Copy)]
pub struct ST {
    pub s: f32,
    pub t: f32,
}

impl ST {
    pub fn from_lc(lc: LC) -> Self {
        Self {
            s: lc.chroma / lc.lightness,
            t: lc.chroma / (1.0 - lc.lightness),
        }
    }

    fn mid(a_: f32, b_: f32) -> Self {
        let s = 0.115_169_93
            + 1.0
                / (7.447_789_7
                    + 4.159_012_3 * b_
                    + a_ * (-2.195_573_6
                        + 1.751_984 * b_
                        + a_ * (-2.137_049_4 - 10.023_01 * b_
                            + a_ * (-4.248_946 + 5.387_708 * b_ + 4.698_910_2 * a_))));
        let t = 0.112_396_42
            + 1.0
                / (1.613_203_2 - 0.681_243_8 * b_
                    + a_ * (0.403_706_13
                        + 0.901_481_2 * b_
                        + a_ * (-0.270_879_42
                            + 0.612_239_9 * b_
                            + a_ * (0.002_992_15 - 0.453_995_67 * b_ - 0.146_618_72 * a_))));
        Self { s, t }
    }
}

pub fn toe(oklab_lightness: f32) -> f32 {
    let k_1 = 0.206;
    let k_2 = 0.03;
    let k_3 = (1.0 + k_1) / (1.0 + k_2);
    0.5 * (k_3 * oklab_lightness - k_1
        + ((k_3 * oklab_lightness - k_1).powi(2) + 4.0 * k_2 * k_3 * oklab_lightness).sqrt())
}

pub fn toe_inv(l_r: f32) -> f32 {
    let k_1 = 0.206;
    let k_2 = 0.03;
    let k_3 = (1.0 + k_1) / (1.0 + k_2);
    (l_r * l_r + k_1 * l_r) / (k_3 * (l_r + k_2))
}

pub fn find_cusp(a: f32, b: f32) -> LC {
    let max_saturation = max_saturation(a, b);
    let rgb_at_max = oklab_to_linear_srgb(OkLab::new(1.0, max_saturation * a, max_saturation * b));
    let max_lightness = (1.0 / rgb_at_max.0.max(rgb_at_max.1).max(rgb_at_max.2)).cbrt();

    LC {
        lightness: max_lightness,
        chroma: max_lightness * max_saturation,
    }
}

fn find_gamut_intersection(a: f32, b: f32, l1: f32, c1: f32, l0: f32, cusp: LC) -> f32 {
    if (l1 - l0) * cusp.chroma - (cusp.lightness - l0) * c1 <= 0.0 {
        return cusp.chroma * l0 / (c1 * cusp.lightness + cusp.chroma * (l0 - l1));
    }

    let t = cusp.chroma * (l0 - 1.0) / (c1 * (cusp.lightness - 1.0) + cusp.chroma * (l0 - l1));
    let dl = l1 - l0;
    let dc = c1;
    let k_l = 0.396_337_78 * a + 0.215_803_76 * b;
    let k_m = -0.105_561_346 * a - 0.063_854_17 * b;
    let k_s = -0.089_484_18 * a - 1.291_485_5 * b;
    let l_dt = dl + dc * k_l;
    let m_dt = dl + dc * k_m;
    let s_dt = dl + dc * k_s;
    let lightness = l0 * (1.0 - t) + t * l1;
    let chroma = t * c1;
    let l_ = lightness + chroma * k_l;
    let m_ = lightness + chroma * k_m;
    let s_ = lightness + chroma * k_s;
    let l = l_.powi(3);
    let m = m_.powi(3);
    let s = s_.powi(3);
    let ldt = 3.0 * l_dt * l_.powi(2);
    let mdt = 3.0 * m_dt * m_.powi(2);
    let sdt = 3.0 * s_dt * s_.powi(2);
    let ldt2 = 6.0 * l_dt.powi(2) * l_;
    let mdt2 = 6.0 * m_dt.powi(2) * m_;
    let sdt2 = 6.0 * s_dt.powi(2) * s_;
    let t_r = halley_step(
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s - 1.0,
        4.076_741_7 * ldt - 3.307_711_6 * mdt + 0.230_969_94 * sdt,
        4.076_741_7 * ldt2 - 3.307_711_6 * mdt2 + 0.230_969_94 * sdt2,
    );
    let t_g = halley_step(
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s - 1.0,
        -1.268_438 * ldt + 2.609_757_4 * mdt - 0.341_319_4 * sdt,
        -1.268_438 * ldt2 + 2.609_757_4 * mdt2 - 0.341_319_4 * sdt2,
    );
    let t_b = halley_step(
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s - 1.0,
        -0.004_196_086_3 * ldt - 0.703_418_6 * mdt + 1.707_614_7 * sdt,
        -0.004_196_086_3 * ldt2 - 0.703_418_6 * mdt2 + 1.707_614_7 * sdt2,
    );

    t + t_r.min(t_g).min(t_b)
}

fn halley_step(f: f32, f1: f32, f2: f32) -> f32 {
    let u = f1 / (f1 * f1 - 0.5 * f * f2);
    if u >= 0.0 { -f * u } else { 1_000_000.0 }
}

fn max_saturation(a: f32, b: f32) -> f32 {
    let (k0, k1, k2, k3, k4, wl, wm, ws) = if -1.881_703_3 * a - 0.809_364_9 * b > 1.0 {
        (
            1.190_862_8,
            1.765_767_3,
            0.596_626_4,
            0.755_152,
            0.567_712_4,
            4.076_741_7,
            -3.307_711_6,
            0.230_969_94,
        )
    } else if 1.814_441 * a - 1.194_452_8 * b > 1.0 {
        (
            0.739_565_13,
            -0.459_544_03,
            0.082_854_27,
            0.125_410_7,
            0.145_032_05,
            -1.268_438,
            2.609_757_4,
            -0.341_319_4,
        )
    } else {
        (
            1.357_336_5,
            -0.009_157_99,
            -1.151_302_1,
            -0.505_596_04,
            0.006_921_67,
            -0.004_196_086_3,
            -0.703_418_6,
            1.707_614_7,
        )
    };

    let mut saturation = k0 + k1 * a + k2 * b + k3 * a.powi(2) + k4 * a * b;
    let k_l = 0.396_337_78 * a + 0.215_803_76 * b;
    let k_m = -0.105_561_346 * a - 0.063_854_17 * b;
    let k_s = -0.089_484_18 * a - 1.291_485_5 * b;
    let l_ = 1.0 + saturation * k_l;
    let m_ = 1.0 + saturation * k_m;
    let s_ = 1.0 + saturation * k_s;
    let l = l_.powi(3);
    let m = m_.powi(3);
    let s = s_.powi(3);
    let l_ds = 3.0 * k_l * l_.powi(2);
    let m_ds = 3.0 * k_m * m_.powi(2);
    let s_ds = 3.0 * k_s * s_.powi(2);
    let l_ds2 = 6.0 * k_l.powi(2) * l_;
    let m_ds2 = 6.0 * k_m.powi(2) * m_;
    let s_ds2 = 6.0 * k_s.powi(2) * s_;
    let f = wl * l + wm * m + ws * s;
    let f1 = wl * l_ds + wm * m_ds + ws * s_ds;
    let f2 = wl * l_ds2 + wm * m_ds2 + ws * s_ds2;
    saturation -= f * f1 / (f1.powi(2) - 0.5 * f * f2);
    saturation
}

pub fn oklab_to_linear_srgb(oklab: OkLab) -> (f32, f32, f32) {
    let l_ = oklab.l + 0.396_337_777_4 * oklab.a + 0.215_803_757_3 * oklab.b;
    let m_ = oklab.l - 0.105_561_345_8 * oklab.a - 0.063_854_172_8 * oklab.b;
    let s_ = oklab.l - 0.089_484_177_5 * oklab.a - 1.291_485_548 * oklab.b;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    (
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    )
}
