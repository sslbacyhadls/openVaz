/// Серийные калибровки Января 7.2 Евро-2, Итэлмы М74 и Bosch ME17.9.7 первой Калины люкс на 21126.
///
/// Идентификаторы и состав датчиков взяты из открытых списков серийного ПО.
/// У Января буква I — заводской блок Итэлмы, та же калибровка Автэла начинается на A.
/// Байты карт зажигания и топливоподачи сюда не копируются.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    /// Всегда есть у этой прошивки.
    Always,
    /// Датчик фаз. У классики I226FM10 его нет.
    Phase,
    /// Датчик детонации. У I226FM10 его нет.
    Knock,
    /// Второй датчик кислорода, после катализатора. Есть у Евро-4 ME17 и Евро-5 М74.
    RearO2,
    /// Регулятор холостого хода. На электронной педали его нет.
    CableIdle,
    /// Положение педали газа. Есть у электронной педали ME17.9.7.
    Pedal,
}

#[derive(Clone, Copy, Debug)]
pub struct Channel {
    pub key: &'static str,
    pub title: &'static str,
    pub unit: &'static str,
    pub decimals: u8,
    pub kind: ChannelKind,
}

pub const CHANNELS: &[Channel] = &[
    Channel { key: "обороты", title: "Обороты", unit: "об/мин", decimals: 0, kind: ChannelKind::Always },
    Channel { key: "обороты_хх", title: "Желаемые обороты холостого хода", unit: "об/мин", decimals: 0, kind: ChannelKind::Always },
    Channel { key: "скорость", title: "Скорость", unit: "км/ч", decimals: 0, kind: ChannelKind::Always },
    Channel { key: "педаль", title: "Педаль газа", unit: "%", decimals: 0, kind: ChannelKind::Pedal },
    Channel { key: "дроссель", title: "Дроссельная заслонка", unit: "%", decimals: 0, kind: ChannelKind::Always },
    Channel { key: "воздух", title: "Температура воздуха", unit: "°C", decimals: 0, kind: ChannelKind::Always },
    Channel { key: "дмрв", title: "Расход воздуха", unit: "кг/ч", decimals: 1, kind: ChannelKind::Always },
    Channel { key: "впрыск", title: "Время впрыска", unit: "мс", decimals: 2, kind: ChannelKind::Always },
    Channel { key: "зажигание", title: "Угол опережения", unit: "°", decimals: 1, kind: ChannelKind::Always },
    Channel { key: "напряжение", title: "Напряжение бортсети", unit: "В", decimals: 1, kind: ChannelKind::Always },
    Channel { key: "коррекция", title: "Коррекция впрыска", unit: "", decimals: 2, kind: ChannelKind::Always },
    Channel { key: "кислород", title: "Датчик кислорода", unit: "В", decimals: 2, kind: ChannelKind::Always },
    Channel { key: "кислород2", title: "Второй датчик кислорода", unit: "В", decimals: 2, kind: ChannelKind::RearO2 },
    Channel { key: "рхх", title: "Регулятор холостого хода", unit: "шаги", decimals: 0, kind: ChannelKind::CableIdle },
    Channel { key: "продувка", title: "Продувка адсорбера", unit: "%", decimals: 0, kind: ChannelKind::Always },
    Channel { key: "нагрузка", title: "Нагрузка", unit: "%", decimals: 0, kind: ChannelKind::Always },
    Channel { key: "расход_час", title: "Расход топлива", unit: "л/ч", decimals: 2, kind: ChannelKind::Always },
    Channel { key: "расход_путь", title: "Путевой расход", unit: "л/100 км", decimals: 1, kind: ChannelKind::Always },
    Channel { key: "фаза", title: "Датчик фаз", unit: "", decimals: 0, kind: ChannelKind::Phase },
    Channel { key: "детонация", title: "Отвод угла по детонации", unit: "°", decimals: 1, kind: ChannelKind::Knock },
];

#[derive(Clone, Copy, Debug)]
pub struct Firmware {
    pub id: &'static str,
    pub vendor: &'static str,
    pub ecu_number: &'static str,
    pub engine: &'static str,
    pub family: &'static str,
    pub note: &'static str,
    pub peak_nm: f32,
    pub rev_limit: f32,
    pub idle_rpm: f32,
    pub final_drive: f32,
    pub displacement_l: f32,
    pub volumetric: f32,
    pub injector_gps: f32,
    pub has_phase: bool,
    pub has_knock: bool,
    pub has_rear_o2: bool,
    /// Электронная педаль: заслонка держит холостой ход сама, регулятора холостого хода нет.
    pub electronic_throttle: bool,
    pub has_ac: bool,
    /// I204DM53 переобогащает смесь на сбросе газа. В I204DO57 это исправлено.
    pub tip_out_rich: bool,
    pub fan_on: f32,
    pub fan_off: f32,
}

impl Firmware {
    pub fn channels(self) -> impl Iterator<Item = &'static Channel> {
        CHANNELS.iter().filter(move |channel| match channel.kind {
            ChannelKind::Always => true,
            ChannelKind::Phase => self.has_phase,
            ChannelKind::Knock => self.has_knock,
            ChannelKind::RearO2 => self.has_rear_o2,
            ChannelKind::CableIdle => !self.electronic_throttle,
            ChannelKind::Pedal => self.electronic_throttle,
        })
    }

    pub fn has_channel(self, key: &str) -> bool {
        self.channels().any(|channel| channel.key == key)
    }
}

pub fn catalog() -> &'static [Firmware] {
    &FIRMWARES
}

pub fn find(name: &str) -> Option<&'static Firmware> {
    let name = name.trim();
    FIRMWARES.iter().find(|firmware| {
        firmware.id.eq_ignore_ascii_case(name)
            || (firmware.family == "Январь 7.2" && avtel_twin(firmware.id).eq_ignore_ascii_case(name))
    })
}

pub fn default_firmware() -> &'static Firmware {
    find("I201CO58").expect("калибровка Калины")
}

/// Итэлма (I) и Автэл (A) выпускали одну и ту же калибровку под разными буквами.
fn avtel_twin(itelma_id: &str) -> String {
    let mut twin = String::from("A");
    twin.push_str(itelma_id.trim_start_matches(['I', 'i']));
    twin
}

const FIRMWARES: [Firmware; 8] = [
    Firmware {
        id: "I201CO57",
        vendor: "Итэлма",
        ecu_number: "11183-1411020-22",
        engine: "11183",
        family: "Январь 7.2",
        note: "Калина 1,6 8V, Евро-2. Один порог вентилятора охлаждения. Пара Автэла — A201CO57.",
        peak_nm: 120.0,
        rev_limit: 5600.0,
        idle_rpm: 850.0,
        final_drive: 3.7,
        displacement_l: 1.6,
        volumetric: 0.84,
        injector_gps: 1.55,
        has_phase: true,
        has_knock: true,
        has_rear_o2: false,
        electronic_throttle: false,
        has_ac: true,
        tip_out_rich: false,
        fan_on: 99.0,
        fan_off: 97.0,
    },
    Firmware {
        id: "I201CO58",
        vendor: "Итэлма",
        ecu_number: "11183-1411020-22",
        engine: "11183",
        family: "Январь 7.2",
        note: "Калина 1,6 8V, Евро-2. Отличие от I201CO57 — раздельные пороги включения и выключения вентилятора. Пара Автэла — A201CO58.",
        peak_nm: 120.0,
        rev_limit: 5600.0,
        idle_rpm: 850.0,
        final_drive: 3.7,
        displacement_l: 1.6,
        volumetric: 0.84,
        injector_gps: 1.55,
        has_phase: true,
        has_knock: true,
        has_rear_o2: false,
        electronic_throttle: false,
        has_ac: true,
        tip_out_rich: false,
        fan_on: 101.0,
        fan_off: 94.0,
    },
    Firmware {
        id: "I204DM53",
        vendor: "Итэлма",
        ecu_number: "21114-1411020-32",
        engine: "21114",
        family: "Январь 7.2",
        note: "21114 1,6 8V, Евро-2. На сбросе газа смесь ненадолго переобогащается. Пара Автэла — A204DM53.",
        peak_nm: 120.0,
        rev_limit: 5600.0,
        idle_rpm: 850.0,
        final_drive: 3.7,
        displacement_l: 1.6,
        volumetric: 0.84,
        injector_gps: 1.55,
        has_phase: true,
        has_knock: true,
        has_rear_o2: false,
        electronic_throttle: false,
        has_ac: false,
        tip_out_rich: true,
        fan_on: 102.0,
        fan_off: 96.0,
    },
    Firmware {
        id: "I204DO57",
        vendor: "Итэлма",
        ecu_number: "21114-1411020-32",
        engine: "21114",
        family: "Январь 7.2",
        note: "21114 1,6 8V, Евро-2. Ошибка переобогащения на сбросе газа исправлена. Пара Автэла — A204DO57.",
        peak_nm: 120.0,
        rev_limit: 5600.0,
        idle_rpm: 850.0,
        final_drive: 3.7,
        displacement_l: 1.6,
        volumetric: 0.84,
        injector_gps: 1.55,
        has_phase: true,
        has_knock: true,
        has_rear_o2: false,
        electronic_throttle: false,
        has_ac: false,
        tip_out_rich: false,
        fan_on: 102.0,
        fan_off: 96.0,
    },
    Firmware {
        id: "I205DO54",
        vendor: "Итэлма",
        ecu_number: "21124-1411020-32",
        engine: "21124",
        family: "Январь 7.2",
        note: "21124 1,6 16V, Евро-2. Выше наполнение и отсечка, короче импульс форсунки. Пара Автэла — A205DO54.",
        peak_nm: 131.0,
        rev_limit: 6200.0,
        idle_rpm: 840.0,
        final_drive: 3.7,
        displacement_l: 1.6,
        volumetric: 0.93,
        injector_gps: 2.15,
        has_phase: true,
        has_knock: true,
        has_rear_o2: false,
        electronic_throttle: false,
        has_ac: false,
        tip_out_rich: false,
        fan_on: 102.0,
        fan_off: 96.0,
    },
    Firmware {
        id: "I464SG04",
        vendor: "Итэлма",
        ecu_number: "21126-1411020-79",
        engine: "21126",
        family: "Итэлма М74",
        note: "Штатный блок М74, Евро-5, CAN. Приора, Приора 2, Гранта, Калина 2, МКПП, 1.6 16V. ДМРВ и два датчика кислорода, момент выше, чем у 21124.",
        peak_nm: 145.0,
        rev_limit: 6400.0,
        idle_rpm: 840.0,
        final_drive: 3.7,
        displacement_l: 1.6,
        volumetric: 0.96,
        injector_gps: 2.25,
        has_phase: true,
        has_knock: true,
        has_rear_o2: true,
        electronic_throttle: false,
        has_ac: true,
        tip_out_rich: false,
        fan_on: 103.0,
        fan_off: 97.0,
    },
    Firmware {
        id: "B564CD04",
        vendor: "Bosch",
        ecu_number: "21126-1411020-50",
        engine: "21126",
        family: "Bosch ME17.9.7",
        note: "Калина 1, комплектация люкс, 1.6 16V, Евро-4, 28.10.2011. Электронная педаль, ДМРВ и два датчика кислорода. Более ранняя Евро-4 того же блока — B564CA02.",
        peak_nm: 145.0,
        rev_limit: 6200.0,
        idle_rpm: 840.0,
        final_drive: 3.7,
        displacement_l: 1.6,
        volumetric: 0.96,
        injector_gps: 2.25,
        has_phase: true,
        has_knock: true,
        has_rear_o2: true,
        electronic_throttle: true,
        has_ac: true,
        tip_out_rich: false,
        fan_on: 102.0,
        fan_off: 96.0,
    },
    Firmware {
        id: "I226FM10",
        vendor: "Итэлма",
        ecu_number: "21067-1411020-12",
        engine: "21067",
        family: "Январь 7.2",
        note: "Классика 1,6, Евро-2. Без датчика фаз и без датчика детонации, кондиционера нет. Пара Автэла — A226FM10.",
        peak_nm: 110.0,
        rev_limit: 5200.0,
        idle_rpm: 900.0,
        final_drive: 3.9,
        displacement_l: 1.6,
        volumetric: 0.80,
        injector_gps: 1.45,
        has_phase: false,
        has_knock: false,
        has_rear_o2: false,
        electronic_throttle: false,
        has_ac: false,
        tip_out_rich: false,
        fan_on: 102.0,
        fan_off: 96.0,
    },
];

pub fn format_channel(channel: &Channel, value: f32) -> String {
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
    fn avtel_names_find_the_itelma_calibration() {
        assert_eq!(find("A201CO58").unwrap().id, "I201CO58");
        assert_eq!(find("i205do54").unwrap().engine, "21124");
        assert!(find("нет такой").is_none());
    }

    #[test]
    fn classic_firmware_drops_phase_and_knock() {
        let classic = find("I226FM10").unwrap();
        let sixteen = find("I205DO54").unwrap();
        assert!(!classic.has_channel("фаза"));
        assert!(!classic.has_channel("детонация"));
        assert!(sixteen.has_channel("фаза"));
        assert!(sixteen.has_channel("детонация"));
        assert!(classic.has_channel("обороты"));
    }

    #[test]
    fn itelma_m74_is_the_21126_block() {
        let block = find("I464SG04").unwrap();
        assert_eq!(block.family, "Итэлма М74");
        assert_eq!(block.engine, "21126");
        assert_eq!(block.ecu_number, "21126-1411020-79");
        assert!(block.has_channel("кислород2"));
        assert!(block.has_channel("дмрв"));
        assert!(!find("I205DO54").unwrap().has_channel("кислород2"));
        assert!(block.peak_nm > find("I205DO54").unwrap().peak_nm);
        assert!(block.rev_limit > find("I205DO54").unwrap().rev_limit);
        assert!(find("A464SG04").is_none());
    }

    #[test]
    fn first_kalina_lux_21126_uses_electronic_throttle() {
        let lux = find("B564CD04").unwrap();
        assert_eq!(lux.family, "Bosch ME17.9.7");
        assert_eq!(lux.vendor, "Bosch");
        assert_eq!(lux.engine, "21126");
        assert_eq!(lux.ecu_number, "21126-1411020-50");
        assert!(lux.has_ac);
        assert!(lux.has_channel("педаль"));
        assert!(lux.has_channel("кислород2"));
        assert!(lux.has_channel("дмрв"));
        assert!(!lux.has_channel("рхх"));
        assert!(find("I201CO58").unwrap().has_channel("рхх"));
    }
}
