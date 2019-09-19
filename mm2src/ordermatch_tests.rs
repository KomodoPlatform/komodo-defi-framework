use common::mm_ctx::{MmArc, MmCtxBuilder};
use mocktopus::mocking::*;
use super::*;

#[test]
fn test_match_maker_order_and_taker_request() {
    let maker = MakerOrder {
        base: "BASE".into(),
        rel: "REL".into(),
        created_at: now_ms(),
        max_base_vol: 10.into(),
        max_base_vol_rat: BigRational::from_integer(10.into()),
        min_base_vol: 0.into(),
        min_base_vol_rat: BigRational::from_integer(0.into()),
        price: 1.into(),
        price_rat: BigRational::from_integer(1.into()),
        matches: HashMap::new(),
        started_swaps: Vec::new(),
        uuid: Uuid::new_v4(),
    };

    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid: Uuid::new_v4(),
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 10.into(),
        base_amount_rat: Some(BigRational::from_integer(10.into())),
        rel_amount: 20.into(),
        rel_amount_rat: Some(BigRational::from_integer(20.into())),
        action: TakerAction::Buy,
    };

    let actual = match_order_and_request(&maker, &request);
    let expected = OrderMatchResult::Matched((10.into(), 10.into()));
    assert_eq!(expected, actual);

    let maker = MakerOrder {
        base: "BASE".into(),
        rel: "REL".into(),
        created_at: now_ms(),
        max_base_vol: 10.into(),
        max_base_vol_rat: BigRational::from_integer(10.into()),
        min_base_vol: 0.into(),
        min_base_vol_rat: BigRational::from_integer(0.into()),
        price: "0.5".parse().unwrap(),
        price_rat: BigRational::new(1.into(), 2.into()),
        matches: HashMap::new(),
        started_swaps: Vec::new(),
        uuid: Uuid::new_v4(),
    };

    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid: Uuid::new_v4(),
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 10.into(),
        base_amount_rat: Some(BigRational::from_integer(10.into())),
        rel_amount: 20.into(),
        rel_amount_rat: Some(BigRational::from_integer(20.into())),
        action: TakerAction::Buy,
    };

    let actual = match_order_and_request(&maker, &request);
    let expected = OrderMatchResult::Matched((10.into(), 5.into()));
    assert_eq!(expected, actual);

    let maker = MakerOrder {
        base: "BASE".into(),
        rel: "REL".into(),
        created_at: now_ms(),
        max_base_vol: 10.into(),
        max_base_vol_rat: BigRational::from_integer(10.into()),
        min_base_vol: 0.into(),
        min_base_vol_rat: BigRational::from_integer(0.into()),
        price: "0.5".parse().unwrap(),
        price_rat: BigRational::new(1.into(), 2.into()),
        matches: HashMap::new(),
        started_swaps: Vec::new(),
        uuid: Uuid::new_v4(),
    };

    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid: Uuid::new_v4(),
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 10.into(),
        base_amount_rat: Some(BigRational::from_integer(10.into())),
        rel_amount: 2.into(),
        rel_amount_rat: Some(BigRational::from_integer(2.into())),
        action: TakerAction::Buy,
    };

    let actual = match_order_and_request(&maker, &request);
    let expected = OrderMatchResult::NotMatched;
    assert_eq!(expected, actual);

    let maker = MakerOrder {
        base: "BASE".into(),
        rel: "REL".into(),
        created_at: now_ms(),
        max_base_vol: 10.into(),
        max_base_vol_rat: BigRational::from_integer(10.into()),
        min_base_vol: 0.into(),
        min_base_vol_rat: BigRational::from_integer(0.into()),
        price: "0.5".parse().unwrap(),
        price_rat: BigRational::new(1.into(), 2.into()),
        matches: HashMap::new(),
        started_swaps: Vec::new(),
        uuid: Uuid::new_v4(),
    };

    let request = TakerRequest {
        base: "REL".into(),
        rel: "BASE".into(),
        uuid: Uuid::new_v4(),
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 5.into(),
        base_amount_rat: Some(BigRational::from_integer(5.into())),
        rel_amount: 10.into(),
        rel_amount_rat: Some(BigRational::from_integer(10.into())),
        action: TakerAction::Sell,
    };

    let actual = match_order_and_request(&maker, &request);
    let expected = OrderMatchResult::Matched((10.into(), 5.into()));
    assert_eq!(expected, actual);

    let maker = MakerOrder {
        base: "BASE".into(),
        rel: "REL".into(),
        created_at: now_ms(),
        max_base_vol: 20.into(),
        max_base_vol_rat: BigRational::from_integer(20.into()),
        min_base_vol: 0.into(),
        min_base_vol_rat: BigRational::from_integer(0.into()),
        price: "0.5".parse().unwrap(),
        price_rat: BigRational::new(1.into(), 2.into()),
        matches: HashMap::new(),
        started_swaps: Vec::new(),
        uuid: Uuid::new_v4(),
    };

    let request = TakerRequest {
        base: "REL".into(),
        rel: "BASE".into(),
        uuid: Uuid::new_v4(),
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 10.into(),
        base_amount_rat: Some(BigRational::from_integer(10.into())),
        rel_amount: 10.into(),
        rel_amount_rat: Some(BigRational::from_integer(10.into())),
        action: TakerAction::Sell,
    };

    let actual = match_order_and_request(&maker, &request);
    let expected = OrderMatchResult::Matched((20.into(), 10.into()));
    assert_eq!(expected, actual);

    let maker = MakerOrder {
        base: "BASE".into(),
        rel: "REL".into(),
        created_at: now_ms(),
        max_base_vol: 1.into(),
        max_base_vol_rat: BigRational::from_integer(1.into()),
        min_base_vol: 0.into(),
        min_base_vol_rat: BigRational::from_integer(0.into()),
        price: "1".parse().unwrap(),
        price_rat: BigRational::from_integer(1.into()),
        matches: HashMap::new(),
        started_swaps: Vec::new(),
        uuid: Uuid::new_v4(),
    };

    let request = TakerRequest {
        base: "REL".into(),
        rel: "BASE".into(),
        uuid: Uuid::new_v4(),
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 1.into(),
        base_amount_rat: Some(BigRational::from_integer(1.into())),
        rel_amount: "0.9".parse().unwrap(),
        rel_amount_rat: Some(BigRational::new(9.into(), 10.into())),
        action: TakerAction::Sell,
    };

    let actual = match_order_and_request(&maker, &request);
    let expected = OrderMatchResult::Matched((1.into(), 1.into()));
    assert_eq!(expected, actual);
}

#[test]
fn test_maker_order_available_amount() {
    let mut maker = MakerOrder {
        base: "BASE".into(),
        rel: "REL".into(),
        created_at: now_ms(),
        max_base_vol: 10.into(),
        max_base_vol_rat: BigRational::from_integer(10.into()),
        min_base_vol: 0.into(),
        min_base_vol_rat: BigRational::from_integer(0.into()),
        price: 1.into(),
        price_rat: BigRational::from_integer(1.into()),
        matches: HashMap::new(),
        started_swaps: Vec::new(),
        uuid: Uuid::new_v4(),
    };
    maker.matches.insert(Uuid::new_v4(), MakerMatch {
        request: TakerRequest {
            uuid: Uuid::new_v4(),
            base: "BASE".into(),
            rel: "REL".into(),
            base_amount: 5.into(),
            base_amount_rat: None,
            rel_amount: 5.into(),
            rel_amount_rat: None,
            sender_pubkey: H256Json::default(),
            dest_pub_key: H256Json::default(),
            method: "request".into(),
            action: TakerAction::Buy,
        },
        reserved: MakerReserved {
            method: "reserved".into(),
            base: "BASE".into(),
            rel: "REL".into(),
            base_amount: 5.into(),
            base_amount_rat: Some(BigRational::from_integer(5.into())),
            rel_amount: 5.into(),
            rel_amount_rat: Some(BigRational::from_integer(5.into())),
            sender_pubkey: H256Json::default(),
            dest_pub_key: H256Json::default(),
            maker_order_uuid: Uuid::new_v4(),
            taker_order_uuid: Uuid::new_v4(),
        },
        connect: None,
        connected: None,
        last_updated: now_ms(),
    });
    maker.matches.insert(Uuid::new_v4(), MakerMatch {
        request: TakerRequest {
            uuid: Uuid::new_v4(),
            base: "BASE".into(),
            rel: "REL".into(),
            base_amount: 1.into(),
            base_amount_rat: Some(BigRational::from_integer(1.into())),
            rel_amount: 1.into(),
            rel_amount_rat: Some(BigRational::from_integer(1.into())),
            sender_pubkey: H256Json::default(),
            dest_pub_key: H256Json::default(),
            method: "request".into(),
            action: TakerAction::Buy,
        },
        reserved: MakerReserved {
            method: "reserved".into(),
            base: "BASE".into(),
            rel: "REL".into(),
            base_amount: 1.into(),
            base_amount_rat: Some(BigRational::from_integer(1.into())),
            rel_amount: 1.into(),
            rel_amount_rat: Some(BigRational::from_integer(1.into())),
            sender_pubkey: H256Json::default(),
            dest_pub_key: H256Json::default(),
            maker_order_uuid: Uuid::new_v4(),
            taker_order_uuid: Uuid::new_v4(),
        },
        connect: None,
        connected: None,
        last_updated: now_ms(),
    });

    let expected = BigRational::from_integer(4.into());
    let actual = maker.available_amount();
    assert_eq!(MmNumber::from(expected), actual);
}

#[test]
fn test_taker_match_reserved() {
    let uuid = Uuid::new_v4();

    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid,
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 10.into(),
        base_amount_rat: Some(BigRational::from_integer(10.into())),
        rel_amount: 10.into(),
        rel_amount_rat: Some(BigRational::from_integer(10.into())),
        action: TakerAction::Buy,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms()
    };

    let reserved = MakerReserved {
        method: "reserved".into(),
        base: "BASE".into(),
        rel: "REL".into(),
        base_amount: 10.into(),
        base_amount_rat: Some(BigRational::from_integer(10.into())),
        rel_amount: 10.into(),
        rel_amount_rat: Some(BigRational::from_integer(10.into())),
        sender_pubkey: H256Json::default(),
        dest_pub_key: H256Json::default(),
        maker_order_uuid: Uuid::new_v4(),
        taker_order_uuid: uuid,
    };

    assert_eq!(MatchReservedResult::Matched, order.match_reserved(&reserved));


    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid,
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 10.into(),
        base_amount_rat: Some(BigRational::from_integer(10.into())),
        rel_amount: 10.into(),
        rel_amount_rat: Some(BigRational::from_integer(10.into())),
        action: TakerAction::Sell,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms()
    };

    let reserved = MakerReserved {
        method: "reserved".into(),
        base: "REL".into(),
        rel: "BASE".into(),
        base_amount: 10.into(),
        base_amount_rat: Some(BigRational::from_integer(10.into())),
        rel_amount: 10.into(),
        rel_amount_rat: Some(BigRational::from_integer(10.into())),
        sender_pubkey: H256Json::default(),
        dest_pub_key: H256Json::default(),
        maker_order_uuid: Uuid::new_v4(),
        taker_order_uuid: uuid,
    };

    assert_eq!(MatchReservedResult::Matched, order.match_reserved(&reserved));

    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid,
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 1.into(),
        base_amount_rat: Some(BigRational::from_integer(1.into())),
        rel_amount: "0.9".parse().unwrap(),
        rel_amount_rat: Some(BigRational::new(9.into(), 10.into())),
        action: TakerAction::Sell,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms()
    };

    let reserved = MakerReserved {
        method: "reserved".into(),
        base: "REL".into(),
        rel: "BASE".into(),
        base_amount: 1.into(),
        base_amount_rat: Some(BigRational::from_integer(1.into())),
        rel_amount: 1.into(),
        rel_amount_rat: Some(BigRational::from_integer(1.into())),
        sender_pubkey: H256Json::default(),
        dest_pub_key: H256Json::default(),
        maker_order_uuid: Uuid::new_v4(),
        taker_order_uuid: uuid,
    };

    assert_eq!(MatchReservedResult::Matched, order.match_reserved(&reserved));

    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid,
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 1.into(),
        base_amount_rat: Some(BigRational::from_integer(1.into())),
        rel_amount: "0.9".parse().unwrap(),
        rel_amount_rat: Some(BigRational::new(9.into(), 10.into())),
        action: TakerAction::Sell,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms()
    };

    let reserved = MakerReserved {
        method: "reserved".into(),
        base: "REL".into(),
        rel: "BASE".into(),
        base_amount: "0.8".parse().unwrap(),
        base_amount_rat: Some(BigRational::new(8.into(), 10.into())),
        rel_amount: 1.into(),
        rel_amount_rat: Some(BigRational::from_integer(1.into())),
        sender_pubkey: H256Json::default(),
        dest_pub_key: H256Json::default(),
        maker_order_uuid: Uuid::new_v4(),
        taker_order_uuid: uuid,
    };

    assert_eq!(MatchReservedResult::NotMatched, order.match_reserved(&reserved));

    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid,
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 1.into(),
        base_amount_rat: Some(BigRational::from_integer(1.into())),
        rel_amount: 2.into(),
        rel_amount_rat: Some(BigRational::from_integer(2.into())),
        action: TakerAction::Buy,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms()
    };

    let reserved = MakerReserved {
        method: "reserved".into(),
        base: "BASE".into(),
        rel: "REL".into(),
        base_amount: 1.into(),
        base_amount_rat: Some(BigRational::from_integer(1.into())),
        rel_amount: 1.into(),
        rel_amount_rat: Some(BigRational::from_integer(1.into())),
        sender_pubkey: H256Json::default(),
        dest_pub_key: H256Json::default(),
        maker_order_uuid: Uuid::new_v4(),
        taker_order_uuid: uuid,
    };

    assert_eq!(MatchReservedResult::Matched, order.match_reserved(&reserved));

    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid,
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 1.into(),
        base_amount_rat: None,
        rel_amount: 2.into(),
        rel_amount_rat: None,
        action: TakerAction::Buy,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms()
    };

    let reserved = MakerReserved {
        method: "reserved".into(),
        base: "BASE".into(),
        rel: "REL".into(),
        base_amount: 1.into(),
        base_amount_rat: Some(BigRational::from_integer(1.into())),
        rel_amount: 1.into(),
        rel_amount_rat: Some(BigRational::from_integer(1.into())),
        sender_pubkey: H256Json::default(),
        dest_pub_key: H256Json::default(),
        maker_order_uuid: Uuid::new_v4(),
        taker_order_uuid: uuid,
    };

    assert_eq!(MatchReservedResult::Matched, order.match_reserved(&reserved));

    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid,
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 1.into(),
        base_amount_rat: Some(BigRational::from_integer(1.into())),
        rel_amount: 2.into(),
        rel_amount_rat: Some(BigRational::from_integer(2.into())),
        action: TakerAction::Buy,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms()
    };

    let reserved = MakerReserved {
        method: "reserved".into(),
        base: "BASE".into(),
        rel: "REL".into(),
        base_amount: 1.into(),
        base_amount_rat: None,
        rel_amount: 1.into(),
        rel_amount_rat: None,
        sender_pubkey: H256Json::default(),
        dest_pub_key: H256Json::default(),
        maker_order_uuid: Uuid::new_v4(),
        taker_order_uuid: uuid,
    };

    assert_eq!(MatchReservedResult::Matched, order.match_reserved(&reserved));

    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid,
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 1.into(),
        base_amount_rat: Some(BigRational::from_integer(1.into())),
        rel_amount: 2.into(),
        rel_amount_rat: Some(BigRational::from_integer(2.into())),
        action: TakerAction::Buy,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms()
    };

    let reserved = MakerReserved {
        method: "reserved".into(),
        base: "BASE".into(),
        rel: "REL".into(),
        base_amount: 1.into(),
        base_amount_rat: Some(BigRational::from_integer(1.into())),
        rel_amount: 3.into(),
        rel_amount_rat: Some(BigRational::from_integer(3.into())),
        sender_pubkey: H256Json::default(),
        dest_pub_key: H256Json::default(),
        maker_order_uuid: Uuid::new_v4(),
        taker_order_uuid: uuid,
    };

    assert_eq!(MatchReservedResult::NotMatched, order.match_reserved(&reserved));

    let order = TakerOrder {
        created_at: 1568358064115,
        request: TakerRequest {
            base: "RICK".into(),
            rel: "MORTY".into(),
            base_amount: "0.3333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333".parse().unwrap(),
            base_amount_rat: Some(BigRational::new(1.into(), 3.into())),
            rel_amount: 1.into(),
            rel_amount_rat: Some(BigRational::from_integer(1.into())),
            action: TakerAction::Buy,
            uuid,
            method: "request".into(),
            sender_pubkey: H256Json::default(),
            dest_pub_key: H256Json::default(),
        },
        matches: HashMap::new(),
    };

    let reserved = MakerReserved {
        base: "RICK".into(),
        rel: "MORTY".into(),
        base_amount: "0.3333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333".parse().unwrap(),
        base_amount_rat: None,
        rel_amount: "0.777777776666666666666666666666666666666666666666666666666666666666666666666666666666666666666666666588888889".parse().unwrap(),
        rel_amount_rat: None,
        taker_order_uuid: uuid,
        maker_order_uuid: uuid,
        method: "reserved".into(),
        sender_pubkey: H256Json::default(),
        dest_pub_key: H256Json::default(),
    };

    assert_eq!(MatchReservedResult::Matched, order.match_reserved(&reserved));
}

#[test]
fn test_taker_order_cancellable() {
    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid: Uuid::new_v4(),
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 1.into(),
        base_amount_rat: Some(BigRational::from_integer(1.into())),
        rel_amount: 2.into(),
        rel_amount_rat: Some(BigRational::from_integer(2.into())),
        action: TakerAction::Buy,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms()
    };

    assert!(order.is_cancellable());

    let request = TakerRequest {
        base: "BASE".into(),
        rel: "REL".into(),
        uuid: Uuid::new_v4(),
        method: "request".into(),
        dest_pub_key: H256Json::default(),
        sender_pubkey: H256Json::default(),
        base_amount: 1.into(),
        base_amount_rat: Some(BigRational::from_integer(1.into())),
        rel_amount: 2.into(),
        rel_amount_rat: Some(BigRational::from_integer(2.into())),
        action: TakerAction::Buy,
    };

    let mut order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms()
    };

    order.matches.insert(
        Uuid::new_v4(),
        TakerMatch {
            last_updated: now_ms(),
            reserved: MakerReserved {
                method: "reserved".into(),
                base: "BASE".into(),
                rel: "REL".into(),
                base_amount: 1.into(),
                base_amount_rat: Some(BigRational::from_integer(1.into())),
                rel_amount: 3.into(),
                rel_amount_rat: Some(BigRational::from_integer(3.into())),
                sender_pubkey: H256Json::default(),
                dest_pub_key: H256Json::default(),
                maker_order_uuid: Uuid::new_v4(),
                taker_order_uuid: Uuid::new_v4(),
            },
            connect: TakerConnect {
                method: "connect".into(),
                sender_pubkey: H256Json::default(),
                dest_pub_key: H256Json::default(),
                maker_order_uuid: Uuid::new_v4(),
                taker_order_uuid: Uuid::new_v4(),
            },
            connected: None,
        }
    );

    assert!(!order.is_cancellable());
}

fn prepare_for_cancel_by(ctx: &MmArc) {
    let ordermatch_ctx = unwrap!(OrdermatchContext::from_ctx(ctx));
    let mut maker_orders = unwrap!(ordermatch_ctx.my_maker_orders.lock());
    let mut taker_orders = unwrap!(ordermatch_ctx.my_taker_orders.lock());

    maker_orders.insert(Uuid::from_bytes([0; 16]), MakerOrder {
        uuid: Uuid::from_bytes([0; 16]),
        base: "ETOMIC".into(),
        rel: "BEER".into(),
        created_at: now_ms(),
        matches: HashMap::new(),
        max_base_vol: 0.into(),
        max_base_vol_rat: BigRational::from_integer(0.into()),
        min_base_vol: 0.into(),
        min_base_vol_rat: BigRational::from_integer(0.into()),
        price: 0.into(),
        price_rat: BigRational::from_integer(0.into()),
        started_swaps: vec![],
    });
    maker_orders.insert(Uuid::from_bytes([1; 16]), MakerOrder {
        uuid: Uuid::from_bytes([1; 16]),
        base: "BEER".into(),
        rel: "ETOMIC".into(),
        created_at: now_ms(),
        matches: HashMap::new(),
        max_base_vol: 0.into(),
        max_base_vol_rat: BigRational::from_integer(0.into()),
        min_base_vol: 0.into(),
        min_base_vol_rat: BigRational::from_integer(0.into()),
        price: 0.into(),
        price_rat: BigRational::from_integer(0.into()),
        started_swaps: vec![],
    });
    maker_orders.insert(Uuid::from_bytes([2; 16]), MakerOrder {
        uuid: Uuid::from_bytes([2; 16]),
        base: "BEER".into(),
        rel: "PIZZA".into(),
        created_at: now_ms(),
        matches: HashMap::new(),
        max_base_vol: 0.into(),
        max_base_vol_rat: BigRational::from_integer(0.into()),
        min_base_vol: 0.into(),
        min_base_vol_rat: BigRational::from_integer(0.into()),
        price: 0.into(),
        price_rat: BigRational::from_integer(0.into()),
        started_swaps: vec![],
    });
    taker_orders.insert(Uuid::from_bytes([3; 16]), TakerOrder {
        matches: HashMap::new(),
        created_at: now_ms(),
        request: TakerRequest {
            base: "ETOMIC".into(),
            rel: "BEER".into(),
            uuid: Uuid::from_bytes([3; 16]),
            action: TakerAction::Buy,
            base_amount: 0.into(),
            base_amount_rat: Some(BigRational::from_integer(0.into())),
            rel_amount: 0.into(),
            rel_amount_rat: Some(BigRational::from_integer(0.into())),
            dest_pub_key: H256Json::default(),
            method: "request".into(),
            sender_pubkey: H256Json::default(),
        }
    });
}

#[test]
fn test_cancel_by_single_coin() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    prepare_for_cancel_by(&ctx);

    delete_my_maker_order.mock_safe(|_, _| {
        MockResult::Return(())
    });
    delete_my_taker_order.mock_safe(|_, _| {
        MockResult::Return(())
    });

    let (cancelled, _) = unwrap!(cancel_orders_by(&ctx, CancelBy::Coin { ticker: "ETOMIC".into() }));
    assert!(cancelled.contains(&Uuid::from_bytes([0; 16])));
    assert!(cancelled.contains(&Uuid::from_bytes([1; 16])));
    assert!(!cancelled.contains(&Uuid::from_bytes([2; 16])));
    assert!(cancelled.contains(&Uuid::from_bytes([3; 16])));
}

#[test]
fn test_cancel_by_pair() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    prepare_for_cancel_by(&ctx);

    delete_my_maker_order.mock_safe(|_, _| {
        MockResult::Return(())
    });
    delete_my_taker_order.mock_safe(|_, _| {
        MockResult::Return(())
    });

    let (cancelled, _) = unwrap!(cancel_orders_by(&ctx, CancelBy::Pair{
        base: "ETOMIC".into(),
        rel: "BEER".into(),
    }));
    assert!(cancelled.contains(&Uuid::from_bytes([0; 16])));
    assert!(!cancelled.contains(&Uuid::from_bytes([1; 16])));
    assert!(!cancelled.contains(&Uuid::from_bytes([2; 16])));
    assert!(cancelled.contains(&Uuid::from_bytes([3; 16])));
}

#[test]
fn test_cancel_by_all() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    prepare_for_cancel_by(&ctx);

    delete_my_maker_order.mock_safe(|_, _| {
        MockResult::Return(())
    });
    delete_my_taker_order.mock_safe(|_, _| {
        MockResult::Return(())
    });

    let (cancelled, _) = unwrap!(cancel_orders_by(&ctx, CancelBy::All));
    assert!(cancelled.contains(&Uuid::from_bytes([0; 16])));
    assert!(cancelled.contains(&Uuid::from_bytes([1; 16])));
    assert!(cancelled.contains(&Uuid::from_bytes([2; 16])));
    assert!(cancelled.contains(&Uuid::from_bytes([3; 16])));
}
