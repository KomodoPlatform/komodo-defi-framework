use std::cell::Cell;

pub(crate) type SmartFractPrecision = (usize, usize);

pub(super) struct SmartFractionFmt {
    precision_min: i32,
    precision_max: i32,
    num: Cell<f64>,
}

#[derive(Debug)]
pub(super) enum SmartFractionTrimErr {
    WrongParams,
}

impl SmartFractionFmt {
    pub(super) fn new(precision_min: usize, precision_max: usize, num: f64) -> Result<Self, SmartFractionTrimErr> {
        if precision_min == 0 || precision_min > precision_max {
            return Err(SmartFractionTrimErr::WrongParams);
        }
        Ok(Self {
            precision_min: precision_min as i32,
            precision_max: precision_max as i32,
            num: Cell::new(num),
        })
    }
}

impl ToString for SmartFractionFmt {
    fn to_string(&self) -> String {
        let num = self.num.get();
        let fraction = if num == 0.0 {
            0
        } else {
            let fruct_order = (num.log10() - 1.0) as i32;
            let fruct_order_abs = fruct_order.abs();
            if fruct_order > 0 {
                self.precision_min
            } else if (self.precision_min + 1..(self.precision_max + 1)).contains(&fruct_order_abs) {
                fruct_order_abs + 1
            } else if fruct_order_abs > self.precision_max {
                self.precision_max
            } else {
                self.precision_min
            }
        };
        let num = (num * 10_f64.powi(fraction)).trunc() / 10_f64.powi(fraction);
        format!("{0:#.1$}", num, fraction as usize)
    }
}

#[test]
fn test_construct_smart_fraction_fmt() {
    assert!(SmartFractionFmt::new(0, 5, 0.0).is_err());
    assert!(SmartFractionFmt::new(5, 2, 0.0).is_err());
}

#[test]
fn test_smart_fraction_fmt() {
    let num = SmartFractionFmt::new(2, 5, 0.0).unwrap();
    assert_eq!(num.to_string(), "0");
    let num = SmartFractionFmt::new(2, 5, 0.1).unwrap();
    assert_eq!(num.to_string(), "0.10");
    let num = SmartFractionFmt::new(2, 5, 0.19909).unwrap();
    assert_eq!(num.to_string(), "0.19");
    let num = SmartFractionFmt::new(2, 5, 0.10001).unwrap();
    assert_eq!(num.to_string(), "0.10");
    let num = SmartFractionFmt::new(2, 5, 0.10991).unwrap();
    assert_eq!(num.to_string(), "0.10");
    let num = SmartFractionFmt::new(2, 5, 0.0011991).unwrap();
    assert_eq!(num.to_string(), "0.0011");
    let num = SmartFractionFmt::new(2, 5, 0.001110000001).unwrap();
    assert_eq!(num.to_string(), "0.0011");
    let num = SmartFractionFmt::new(2, 5, 0.00001700445).unwrap();
    assert_eq!(num.to_string(), "0.000017");
    let num = SmartFractionFmt::new(2, 5, 0.00000199).unwrap();
    assert_eq!(num.to_string(), "0.00000");
    let num = SmartFractionFmt::new(2, 5, 1.0).unwrap();
    assert_eq!(num.to_string(), "1.00");
    let num = SmartFractionFmt::new(2, 5, 1.00001).unwrap();
    assert_eq!(num.to_string(), "1.00");
    let num = SmartFractionFmt::new(2, 5, 1.00000000001).unwrap();
    assert_eq!(num.to_string(), "1.00");
    let num = SmartFractionFmt::new(2, 5, 1.99001).unwrap();
    assert_eq!(num.to_string(), "1.99");
    let num = SmartFractionFmt::new(2, 5, 5000.0).unwrap();
    assert_eq!(num.to_string(), "5000.00");

    let num = SmartFractionFmt::new(1, 5, 0.10991).unwrap();
    assert_eq!(num.to_string(), "0.1");

    let num = SmartFractionFmt::new(2, 2, 0.001110000001).unwrap();
    assert_eq!(num.to_string(), "0.00");
    let num = SmartFractionFmt::new(2, 2, 0.101110000001).unwrap();
    assert_eq!(num.to_string(), "0.10");
}
