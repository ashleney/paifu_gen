#![allow(dead_code)]
use std::array::from_fn;
use std::str::FromStr;

use anyhow::{bail, ensure, Context, Error, Result};
use riichi::convlog::generate::{generate_mjai_logs, Board, Fuurohai, Sutehai};
use riichi::convlog::tenhou::{Log, RawLog};
use riichi::convlog::{mjai_to_tenhou, tenhou_to_mjai};
use riichi::hand::{parse_tile, parse_tiles, tiles_to_string};
use riichi::mjai::Event;
use riichi::state::PlayerState;
use riichi::tile::Tile;
use riichi::tu8;
use serde::Deserialize;
use serde::Serialize;
use serde_json::to_string;
use serde_wasm_bindgen::from_value;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[derive(Deserialize, Serialize)]
pub struct RawBoard {
    pub kyoku: String,
    pub jikaze: String,
    pub kyotaku: String,
    pub honba: String,
    pub dora: String,
    pub scores: Vec<String>,
    pub tehai: String,
    pub kawa: Vec<String>,
    pub fuuro: Vec<String>,
}

impl TryInto<Board> for RawBoard {
    type Error = Error;
    fn try_into(self) -> Result<Board> {
        let mut board = Board::default();

        ensure!(self.kyoku.len() == 2, "kyoku must be <bakaze><honba> (e.g. S3)");
        board.bakaze = Tile::from_str(&self.kyoku[0..1]).context("incorrect bakaze")?;
        board.kyoku = self.kyoku[1..2].parse().context("incorrect kyoku")?;

        board.jikaze = self.jikaze.parse().context("incorrect jikaze")?;
        board.kyotaku = self.kyotaku.parse().context("incorrect kyotaku")?;
        board.honba = self.honba.parse().context("incorrect honba")?;
        board.dora_indicators = parse_tiles(&self.dora).context("incorrect dora")?;
        for (score, raw_score) in board.scores.iter_mut().zip(self.scores) {
            if !raw_score.is_empty() {
                *score = raw_score.parse().context("incorrect score")?;
            } else {
                *score = 25000;
            }
        }
        board.tehai = parse_tiles(&self.tehai).context("incorrect tehai")?;

        // tsumogiri "1p", tedashi "1p.", riichi "1p-"
        for (kawa, raw_kawa) in board.kawa.iter_mut().zip(self.kawa) {
            if raw_kawa.is_empty() {
                continue;
            }
            let mut chars = raw_kawa.chars().peekable();
            while chars.peek().is_some() {
                let tile_string = format!("{}{}", chars.next().unwrap(), chars.next().context("incorrect kawa")?);
                let (tsumogiri, riichi) = match chars.peek() {
                    Some('.') => {
                        chars.next();
                        (true, false)
                    }
                    Some('-') => {
                        chars.next();
                        (true, true)
                    }
                    _ => (false, false),
                };
                kawa.push(Sutehai {
                    pai: parse_tile(&tile_string)?,
                    tedashi: !tsumogiri,
                    riichi,
                });
            }
        }
        // chi (1p)2p3p, pon (1p)1p1p, daiminkan (1p)1p1p1p, ankan 1p1p1p1p, pon+kakan 1p1p(1p)(1p)
        for (fuuro, raw_fuuro) in board.fuuro.iter_mut().zip(self.fuuro) {
            if raw_fuuro.is_empty() {
                continue;
            }
            let mut fuuro_iter = raw_fuuro.chars().peekable();
            let mut in_parentheses = false;
            loop {
                match fuuro_iter.peek() {
                    Some('(') => {
                        _ = fuuro_iter.next();
                        if in_parentheses {
                            bail!("nested opening parenthesis in fuuro");
                        }
                        in_parentheses = true;
                    }
                    Some(')') => {
                        _ = fuuro_iter.next();
                        if !in_parentheses {
                            bail!("extra closing parenthesis in fuuro");
                        }
                        in_parentheses = false;
                    }
                    Some(_) => {
                        let tile_string = format!(
                            "{}{}",
                            fuuro_iter.next().unwrap(),
                            fuuro_iter.next().context("incorrect fuuro")?
                        );
                        fuuro.tiles.push(Fuurohai {
                            tile: parse_tile(&tile_string)?,
                            sideways: in_parentheses,
                        });
                    }
                    None => break,
                }
            }
        }
        Ok(board)
    }
}

#[derive(Serialize)]
struct GenerateResult {
    tenhou_log: String,
    human_tenhou_log: String,
    mjai_log: Vec<Event>,
    player_id: i32,
}

#[wasm_bindgen]
pub fn generate_logs_js(val: JsValue) -> Result<JsValue, JsValue> {
    let raw_board: RawBoard = from_value(val).map_err(|e| JsValue::from_str(&format!("deserialize error: {e}")))?;
    let board: Board = raw_board
        .try_into()
        .map_err(|e| JsValue::from_str(&format!("parse error: {e}")))?;
    let events = generate_mjai_logs(board).map_err(|e| JsValue::from_str(&format!("log generation error: {e}")))?;
    let raw_tenhou_log = mjai_to_tenhou(&events).map_err(|e| JsValue::from_str(&format!("tenhou conversion error: {e}")))?;
    let tenhou_log_string = to_string(&raw_tenhou_log).map_err(|e| JsValue::from_str(&format!("serialization error: {e}")))?;
    let pretty_tenhou_log = raw_tenhou_log
        .to_string_pretty()
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let player_id = match events.first() {
        Some(Event::StartGame { id: Some(id), .. }) => *id,
        _ => return Err(JsValue::from_str("mjai logs do not start with StartGame")),
    };

    let result = GenerateResult {
        tenhou_log: tenhou_log_string,
        mjai_log: events,
        player_id: player_id as i32,
        human_tenhou_log: pretty_tenhou_log,
    };

    serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&format!("serialize result error: {e}")))
}

pub fn tiles_vec_to_string(tiles: &[Tile]) -> String {
    let mut tiles_34 = [0; 34];
    let mut aka = [false; 3];
    for tile in tiles {
        tiles_34[tile.as_usize()] += 1;
        match tile.as_u8() {
            tu8!(5mr) => aka[0] = true,
            tu8!(5pr) => aka[1] = true,
            tu8!(5sr) => aka[2] = true,
            _ => {}
        }
    }

    tiles_to_string(&tiles_34, aka).replace(" ", "")
}

#[wasm_bindgen]
pub fn generate_board_from_tenhou_js(val: JsValue, jikaze: JsValue) -> Result<JsValue, JsValue> {
    let raw_tenhou_log: RawLog = from_value(val).map_err(|e| JsValue::from_str(&format!("deserialize error: {e}")))?;
    let tenhou_log: Log = raw_tenhou_log
        .try_into()
        .map_err(|e| JsValue::from_str(&format!("deserialize error: {e}")))?;
    if tenhou_log.kyokus.is_empty() {
        return Err(JsValue::from_str("no kyokus"));
    }
    let events = tenhou_to_mjai(&tenhou_log).map_err(|e| JsValue::from_str(&format!("parse error: {e}")))?;

    let jikaze_str = jikaze.as_string().ok_or_else(|| JsValue::from_str("invalid jikaze"))?;
    let jikaze = Tile::from_str(&jikaze_str).map_err(|e| JsValue::from_str(&format!("invalid jikaze: {e}")))?;

    let kyoku = tenhou_log.kyokus[0].meta.kyoku_num;
    let oya = kyoku % 4;
    let player_id = (4 + oya + jikaze.as_u8() - tu8!(E)) % 4;
    // TODO: Do not actually use state to process, use our own
    let mut state = PlayerState::new(player_id as u8);

    let mut visible_kawa: [Vec<(Tile, bool, bool)>; 4] = from_fn(|_| vec![]);
    let mut fuuro: [Vec<Vec<(Tile, bool)>>; 4] = from_fn(|_| vec![]);
    for event in events {
        state
            .update(&event)
            .map_err(|e| JsValue::from_str(&format!("invalid event: {e}")))?;
        match event {
            Event::Dahai { actor, pai, tsumogiri } => {
                visible_kawa[state.rel(actor)].push((pai, tsumogiri, state.riichi_declared[state.rel(actor)]));
            }
            Event::Chi {
                actor,
                target,
                pai,
                consumed,
            }
            | Event::Pon {
                actor,
                target,
                pai,
                consumed,
            } => {
                let mut naki = consumed.map(|tile| (tile, false)).to_vec();
                naki.insert(((4 + actor - target) % 4) as usize - 1, (pai, true));
                fuuro[state.rel(actor)].push(naki);
                visible_kawa[state.rel(target)].pop();
            }
            Event::Daiminkan {
                actor,
                target,
                pai,
                consumed,
            } => {
                let mut naki = consumed.map(|tile| (tile, false)).to_vec();
                let t = ((4 + actor - target) % 4) as usize - 1;
                naki.insert(if t == 2 { 3 } else { t }, (pai, true));
                visible_kawa[state.rel(target)].pop();
            }
            Event::Ankan { actor, consumed } => {
                let naki = consumed.map(|tile| (tile, false)).to_vec();
                fuuro[state.rel(actor)].push(naki);
            }
            Event::Kakan { actor, pai, .. } => {
                'outer: for naki in &mut fuuro[state.rel(actor)] {
                    for (i, (tile, sideways)) in naki.iter().enumerate() {
                        if tile.deaka() == pai.deaka() && *sideways {
                            naki.insert(i + 1, (pai, true));
                            break 'outer;
                        }
                    }
                }
            }
            Event::EndKyoku => break,
            _ => {}
        }
    }

    let kawa_strings = visible_kawa
        .into_iter()
        .map(|actor_kawa| {
            actor_kawa
                .into_iter()
                .map(|(pai, tsumogiri, riichi)| {
                    format!(
                        "{}{}",
                        tiles_vec_to_string(&[pai]),
                        if tsumogiri {
                            "."
                        } else if riichi {
                            "-"
                        } else {
                            ""
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .collect::<Vec<_>>();

    let fuuro_strings = fuuro
        .into_iter()
        .map(|fuuro| {
            fuuro
                .into_iter()
                .rev()
                .flatten()
                .map(|(tile, sideways)| {
                    if sideways {
                        format!("({})", tiles_vec_to_string(&[tile]))
                    } else {
                        tiles_vec_to_string(&[tile]).to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .collect::<Vec<_>>();

    let board = RawBoard {
        kyoku: format!("{}{}", state.bakaze, state.kyoku + 1),
        jikaze: jikaze_str,
        kyotaku: state.kyotaku.to_string(),
        honba: state.honba.to_string(),
        dora: tiles_vec_to_string(&state.dora_indicators),
        scores: state.scores.iter().map(|score| score.to_string()).collect::<Vec<_>>(),
        tehai: tiles_to_string(&state.tehai, state.akas_in_hand).replace(" ", ""),
        kawa: kawa_strings,
        fuuro: fuuro_strings,
    };

    serde_wasm_bindgen::to_value(&board).map_err(|e| JsValue::from_str(&format!("serialize result error: {e}")))
}
