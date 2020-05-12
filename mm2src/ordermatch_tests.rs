use common::{
    mm_ctx::{MmArc, MmCtxBuilder},
    privkey::key_pair_from_seed,
};
use mocktopus::mocking::*;
use std::collections::HashSet;
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
        match_by: MatchBy::Any,
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
        match_by: MatchBy::Any,
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
        match_by: MatchBy::Any,
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
        match_by: MatchBy::Any,
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
        match_by: MatchBy::Any,
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
        match_by: MatchBy::Any,
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
            match_by: MatchBy::Any,
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
            match_by: MatchBy::Any,
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
        match_by: MatchBy::Any,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms(),
        order_type: OrderType::GoodTillCancelled,
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
        match_by: MatchBy::Any,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms(),
        order_type: OrderType::GoodTillCancelled,
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
        match_by: MatchBy::Any,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms(),
        order_type: OrderType::GoodTillCancelled,
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
        match_by: MatchBy::Any,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms(),
        order_type: OrderType::GoodTillCancelled,
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
        match_by: MatchBy::Any,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms(),
        order_type: OrderType::GoodTillCancelled,
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
        match_by: MatchBy::Any,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms(),
        order_type: OrderType::GoodTillCancelled,
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
        match_by: MatchBy::Any,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms(),
        order_type: OrderType::GoodTillCancelled,
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
        match_by: MatchBy::Any,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms(),
        order_type: OrderType::GoodTillCancelled,
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
            match_by: MatchBy::Any,
        },
        matches: HashMap::new(),
        order_type: OrderType::GoodTillCancelled,
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
        match_by: MatchBy::Any,
    };

    let order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms(),
        order_type: OrderType::GoodTillCancelled,
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
        match_by: MatchBy::Any,
    };

    let mut order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms(),
        order_type: OrderType::GoodTillCancelled,
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
            match_by: MatchBy::Any,
        },
        order_type: OrderType::GoodTillCancelled,
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

#[test]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/607
fn test_taker_order_match_by() {
    let uuid = Uuid::new_v4();

    let mut not_matching_uuids = HashSet::new();
    not_matching_uuids.insert(Uuid::new_v4());
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
        match_by: MatchBy::Orders(not_matching_uuids),
    };

    let mut order = TakerOrder {
        request,
        matches: HashMap::new(),
        created_at: now_ms(),
        order_type: OrderType::GoodTillCancelled,
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

    assert_eq!(MatchReservedResult::NotMatched, order.match_reserved(&reserved));

    let mut matching_uuids = HashSet::new();
    matching_uuids.insert(reserved.maker_order_uuid);
    order.request.match_by = MatchBy::Orders(matching_uuids);
    assert_eq!(MatchReservedResult::Matched, order.match_reserved(&reserved));

    let mut not_matching_pubkeys = HashSet::new();
    not_matching_pubkeys.insert([1; 32].into());
    order.request.match_by = MatchBy::Pubkeys(not_matching_pubkeys);
    assert_eq!(MatchReservedResult::NotMatched, order.match_reserved(&reserved));

    let mut matching_pubkeys = HashSet::new();
    matching_pubkeys.insert(H256Json::default());
    order.request.match_by = MatchBy::Pubkeys(matching_pubkeys);
    assert_eq!(MatchReservedResult::Matched, order.match_reserved(&reserved));
}

#[test]
fn lp_connect_start_bob_should_not_be_invoked_if_order_match_already_connected() {
    let order_json = r#"{"max_base_vol":"1","max_base_vol_rat":[[1,[1]],[1,[1]]],"min_base_vol":"0","min_base_vol_rat":[[0,[]],[1,[1]]],"price":"1","price_rat":[[1,[1]],[1,[1]]],"created_at":1589265312093,"base":"ETH","rel":"JST","matches":{"2f9afe84-7a89-4194-8947-45fba563118f":{"request":{"base":"ETH","rel":"JST","base_amount":"0.1","base_amount_rat":[[1,[1]],[1,[10]]],"rel_amount":"0.2","rel_amount_rat":[[1,[1]],[1,[5]]],"action":"Buy","uuid":"2f9afe84-7a89-4194-8947-45fba563118f","method":"request","sender_pubkey":"031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3","dest_pub_key":"0000000000000000000000000000000000000000000000000000000000000000","match_by":{"type":"Any"}},"reserved":{"base":"ETH","rel":"JST","base_amount":"0.1","base_amount_rat":[[1,[1]],[1,[10]]],"rel_amount":"0.1","rel_amount_rat":[[1,[1]],[1,[10]]],"taker_order_uuid":"2f9afe84-7a89-4194-8947-45fba563118f","maker_order_uuid":"5f6516ea-ccaa-453a-9e37-e1c2c0d527e3","method":"reserved","sender_pubkey":"c6a78589e18b482aea046975e6d0acbdea7bf7dbf04d9d5bd67fda917815e3ed","dest_pub_key":"031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3"},"connect":{"taker_order_uuid":"2f9afe84-7a89-4194-8947-45fba563118f","maker_order_uuid":"5f6516ea-ccaa-453a-9e37-e1c2c0d527e3","method":"connect","sender_pubkey":"031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3","dest_pub_key":"c6a78589e18b482aea046975e6d0acbdea7bf7dbf04d9d5bd67fda917815e3ed"},"connected":{"taker_order_uuid":"2f9afe84-7a89-4194-8947-45fba563118f","maker_order_uuid":"5f6516ea-ccaa-453a-9e37-e1c2c0d527e3","method":"connected","sender_pubkey":"c6a78589e18b482aea046975e6d0acbdea7bf7dbf04d9d5bd67fda917815e3ed","dest_pub_key":"031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3"},"last_updated":1589265314408}},"started_swaps":["2f9afe84-7a89-4194-8947-45fba563118f"],"uuid":"5f6516ea-ccaa-453a-9e37-e1c2c0d527e3"}"#;
    let maker_order: MakerOrder = json::from_str(order_json).unwrap();
    let ctx = MmCtxBuilder::default()
        .with_secp256k1_key_pair(key_pair_from_seed("also shoot benefit prefer juice shell elder veteran woman mimic image kidney").unwrap())
        .into_mm_arc();
    let ordermatch_ctx = OrdermatchContext::from_ctx(&ctx).unwrap();
    ordermatch_ctx.my_maker_orders.lock().unwrap().insert(maker_order.uuid, maker_order);

    static mut CONNECT_START_CALLED: bool = false;
    lp_connect_start_bob.mock_safe(|_, _| MockResult::Return(unsafe {
        CONNECT_START_CALLED = true;
    }));

    let connect_json: Json = json::from_str(r#"{"taker_order_uuid":"2f9afe84-7a89-4194-8947-45fba563118f","maker_order_uuid":"5f6516ea-ccaa-453a-9e37-e1c2c0d527e3","method":"connect","sender_pubkey":"031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3","dest_pub_key":"c6a78589e18b482aea046975e6d0acbdea7bf7dbf04d9d5bd67fda917815e3ed"}"#).unwrap();
    lp_trade_command(ctx, connect_json);
    assert!(unsafe { !CONNECT_START_CALLED });
}

#[test]
fn should_process_request_only_once() {
    let order_json = r#"{"max_base_vol":"1","max_base_vol_rat":[[1,[1]],[1,[1]]],"min_base_vol":"0","min_base_vol_rat":[[0,[]],[1,[1]]],"price":"1","price_rat":[[1,[1]],[1,[1]]],"created_at":1589265312093,"base":"ETH","rel":"JST","matches":{"2f9afe84-7a89-4194-8947-45fba563118f":{"request":{"base":"ETH","rel":"JST","base_amount":"0.1","base_amount_rat":[[1,[1]],[1,[10]]],"rel_amount":"0.2","rel_amount_rat":[[1,[1]],[1,[5]]],"action":"Buy","uuid":"2f9afe84-7a89-4194-8947-45fba563118f","method":"request","sender_pubkey":"031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3","dest_pub_key":"0000000000000000000000000000000000000000000000000000000000000000","match_by":{"type":"Any"}},"reserved":{"base":"ETH","rel":"JST","base_amount":"0.1","base_amount_rat":[[1,[1]],[1,[10]]],"rel_amount":"0.1","rel_amount_rat":[[1,[1]],[1,[10]]],"taker_order_uuid":"2f9afe84-7a89-4194-8947-45fba563118f","maker_order_uuid":"5f6516ea-ccaa-453a-9e37-e1c2c0d527e3","method":"reserved","sender_pubkey":"c6a78589e18b482aea046975e6d0acbdea7bf7dbf04d9d5bd67fda917815e3ed","dest_pub_key":"031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3"},"connect":{"taker_order_uuid":"2f9afe84-7a89-4194-8947-45fba563118f","maker_order_uuid":"5f6516ea-ccaa-453a-9e37-e1c2c0d527e3","method":"connect","sender_pubkey":"031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3","dest_pub_key":"c6a78589e18b482aea046975e6d0acbdea7bf7dbf04d9d5bd67fda917815e3ed"},"connected":{"taker_order_uuid":"2f9afe84-7a89-4194-8947-45fba563118f","maker_order_uuid":"5f6516ea-ccaa-453a-9e37-e1c2c0d527e3","method":"connected","sender_pubkey":"c6a78589e18b482aea046975e6d0acbdea7bf7dbf04d9d5bd67fda917815e3ed","dest_pub_key":"031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3"},"last_updated":1589265314408}},"started_swaps":["2f9afe84-7a89-4194-8947-45fba563118f"],"uuid":"5f6516ea-ccaa-453a-9e37-e1c2c0d527e3"}"#;
    let maker_order: MakerOrder = json::from_str(order_json).unwrap();
    let uuid = maker_order.uuid;
    let ctx = MmCtxBuilder::default()
        .with_secp256k1_key_pair(key_pair_from_seed("also shoot benefit prefer juice shell elder veteran woman mimic image kidney").unwrap())
        .into_mm_arc();
    let ordermatch_ctx = OrdermatchContext::from_ctx(&ctx).unwrap();
    ordermatch_ctx.my_maker_orders.lock().unwrap().insert(maker_order.uuid, maker_order);
    let request_json = json!({"base":"ETH","rel":"JST","base_amount":"0.1","base_amount_rat":[[1,[1]],[1,[10]]],"rel_amount":"0.2","rel_amount_rat":[[1,[1]],[1,[5]]],"action":"Buy","uuid":"2f9afe84-7a89-4194-8947-45fba563118f","method":"request","sender_pubkey":"031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3","dest_pub_key":"0000000000000000000000000000000000000000000000000000000000000000","match_by":{"type":"Any"}});
    lp_trade_command(ctx, request_json);
    let maker_orders = ordermatch_ctx.my_maker_orders.lock().unwrap();
    let order = maker_orders.get(&uuid).unwrap();
    // when new request is processed match is replaced with new instance resetting
    // connect and connected to None so by checking is_some we check that request message is ignored
    assert!(order.matches.get(&"2f9afe84-7a89-4194-8947-45fba563118f".parse().unwrap()).unwrap().connect.is_some());
    assert!(order.matches.get(&"2f9afe84-7a89-4194-8947-45fba563118f".parse().unwrap()).unwrap().connected.is_some());
}
