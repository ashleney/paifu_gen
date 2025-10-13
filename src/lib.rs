#![allow(dead_code)]
use std::str::FromStr;

use anyhow::{bail, ensure, Context, Error, Result};
use riichi::convlog::generate::{generate_mjai_logs, Board, Fuurohai, Sutehai};
use riichi::convlog::mjai_to_tenhou;
use riichi::hand::{parse_tile, parse_tiles};
use riichi::mjai::Event;
use riichi::tile::Tile;
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

#[derive(Deserialize)]
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
    let tenhou = mjai_to_tenhou(&events).map_err(|e| JsValue::from_str(&format!("tenhou conversion error: {e}")))?;
    let string = to_string(&tenhou).map_err(|e| JsValue::from_str(&format!("tenhou serialization error: {e}")))?;
    let player_id = match events.first() {
        Some(Event::StartGame { id: Some(id), .. }) => *id,
        _ => return Err(JsValue::from_str("mjai logs do not start with StartGame")),
    };

    let result = GenerateResult {
        tenhou_log: string,
        mjai_log: events,
        player_id: player_id as i32,
    };

    serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&format!("serialize result error: {e}")))
}
