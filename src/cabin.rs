//! Делитель салона: 10 кОм сверху, NTC 10 кОм при 25 °C снизу, бета 3950.
//! Это расчётная бета такого датчика. Замер на машине потом подставит своё число.

const PULLUP_OHM: f32 = 10_000.0;
const NTC_OHM: f32 = 10_000.0;
const BETA: f32 = 3950.0;
const T0_KELVIN: f32 = 298.15;
const VREF: f32 = 3.3;

pub fn cabin_celsius(volts: f32) -> Option<f32> {
    if !(0.05..3.25).contains(&volts) {
        return None;
    }
    let resistance = PULLUP_OHM * volts / (VREF - volts);
    let kelvin = 1.0 / (1.0 / T0_KELVIN + (resistance / NTC_OHM).ln() / BETA);
    Some(kelvin - 273.15)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_divider_points_match_the_beta_equation() {
        let at_25 = cabin_celsius(1.65).unwrap();
        let cold = cabin_celsius(2.2).unwrap();
        let hot = cabin_celsius(1.1).unwrap();
        assert!((at_25 - 25.0).abs() < 0.2, "{at_25}");
        assert!((cold - 10.2).abs() < 0.3, "{cold}");
        assert!((hot - 41.5).abs() < 0.3, "{hot}");
        assert!(cold < at_25 && at_25 < hot);
        assert!(cabin_celsius(0.01).is_none());
        assert!(cabin_celsius(3.29).is_none());
    }
}
