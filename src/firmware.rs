//! Две прошивки, которыми эмулятор заменяет машину: заводская Итэлма М74 и Январь 7.2.
//!
//! Имена и состав датчиков взяты из открытых списков серийного ПО. Карты зажигания
//! и топливоподачи сюда не копируются.

#[derive(Clone, Copy, Debug)]
pub struct Channel {
    pub key: &'static str,
    pub title: &'static str,
    pub unit: &'static str,
    pub decimals: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Firmware {
    pub id: &'static str,
    pub family: &'static str,
    pub calibration: &'static str,
    pub protocol: &'static str,
    pub overheat_code: &'static str,
    pub channels: &'static [Channel],
    pub rev_limit: f32,
    pub idle_rpm: f32,
    pub peak_nm: f32,
    pub fan_on: f32,
    pub fan_off: f32,
}

const SHARED: &[Channel] = &[
    Channel { key: "обороты", title: "Обороты", unit: "об/мин", decimals: 0 },
    Channel { key: "обороты_хх", title: "Обороты холостого хода", unit: "об/мин", decimals: 0 },
    Channel { key: "скорость", title: "Скорость", unit: "км/ч", decimals: 0 },
    Channel { key: "дроссель", title: "Дроссельная заслонка", unit: "%", decimals: 0 },
    Channel { key: "воздух", title: "Температура воздуха", unit: "°C", decimals: 0 },
    Channel { key: "дмрв", title: "Расход воздуха", unit: "кг/ч", decimals: 1 },
    Channel { key: "впрыск", title: "Время впрыска", unit: "мс", decimals: 2 },
    Channel { key: "зажигание", title: "Угол опережения", unit: "°", decimals: 1 },
    Channel { key: "напряжение", title: "Напряжение бортсети", unit: "В", decimals: 1 },
    Channel { key: "коррекция", title: "Коррекция впрыска", unit: "", decimals: 2 },
    Channel { key: "кислород", title: "Датчик кислорода", unit: "В", decimals: 2 },
    Channel { key: "рхх", title: "Регулятор холостого хода", unit: "шаги", decimals: 0 },
    Channel { key: "продувка", title: "Продувка адсорбера", unit: "%", decimals: 0 },
    Channel { key: "детонация", title: "Отвод угла по детонации", unit: "°", decimals: 1 },
    Channel { key: "вентилятор_охлаждения", title: "Вентилятор охлаждения", unit: "", decimals: 0 },
];

const REAR_OXYGEN: Channel = Channel {
    key: "кислород2",
    title: "Второй датчик кислорода",
    unit: "В",
    decimals: 2,
};

const M74_CHANNELS: &[Channel] = &[
    SHARED[0], SHARED[1], SHARED[2], SHARED[3], SHARED[4], SHARED[5], SHARED[6], SHARED[7],
    SHARED[8], SHARED[9], SHARED[10], REAR_OXYGEN, SHARED[11], SHARED[12], SHARED[13], SHARED[14],
];

const JANUARY_CHANNELS: &[Channel] = SHARED;

const FIRMWARES: &[Firmware] = &[
    Firmware {
        id: "итэлма-м74",
        family: "Итэлма М74",
        calibration: "I444CE08",
        protocol: "KWP2000",
        overheat_code: "P0217",
        channels: M74_CHANNELS,
        rev_limit: 5600.0,
        idle_rpm: 850.0,
        peak_nm: 120.0,
        fan_on: 101.0,
        fan_off: 94.0,
    },
    Firmware {
        id: "январь-7.2",
        family: "Январь 7.2",
        calibration: "I201CO58",
        protocol: "KWP2000",
        overheat_code: "P0217",
        channels: JANUARY_CHANNELS,
        rev_limit: 5600.0,
        idle_rpm: 850.0,
        peak_nm: 120.0,
        fan_on: 99.0,
        fan_off: 97.0,
    },
];

pub fn catalog() -> &'static [Firmware] {
    FIRMWARES
}

pub fn find(name: &str) -> Option<&'static Firmware> {
    let name = name.trim();
    FIRMWARES
        .iter()
        .find(|firmware| firmware.id.eq_ignore_ascii_case(name))
}

pub fn default_firmware() -> &'static Firmware {
    find("итэлма-м74").expect("заводская прошивка")
}

impl Firmware {
    pub fn has_channel(self, key: &str) -> bool {
        self.channels.iter().any(|channel| channel.key == key)
    }

    pub fn channel(self, key: &str) -> Option<&'static Channel> {
        self.channels.iter().find(|channel| channel.key == key)
    }
}

pub fn format_value(channel: &Channel, value: f32) -> String {
    match channel.decimals {
        0 => format!("{}", value.round() as i32),
        1 => format!("{value:.1}"),
        _ => format!("{value:.2}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m74_and_january_do_not_share_a_channel_list() {
        let m74 = find("итэлма-м74").unwrap();
        let january = find("январь-7.2").unwrap();
        assert!(m74.has_channel("кислород2"));
        assert!(!january.has_channel("кислород2"));
        assert!(m74.has_channel("обороты") && january.has_channel("обороты"));
        assert_eq!(m74.overheat_code, "P0217");
        assert_eq!(january.overheat_code, "P0217");
        assert_ne!(m74.channels.len(), january.channels.len());
        assert!(find("нет такой").is_none());
        assert_eq!(default_firmware().id, "итэлма-м74");
    }
}
