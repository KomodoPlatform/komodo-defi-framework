//! Seed node metrics
//!

/******************************************************************************
 * Copyright © 2024 Pampex LTD and TillyHK LTD                                *
 *                                                                            *
 * See the CONTRIBUTOR-LICENSE-AGREEMENT, COPYING, LICENSE-COPYRIGHT-NOTICE   *
 * and DEVELOPER-CERTIFICATE-OF-ORIGIN files in the LEGAL directory in        *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * Komodo DeFi Framework software, including this file may be copied, modified, propagated*
 * or distributed except according to the terms contained in the              *
 * LICENSE-COPYRIGHT-NOTICE file.                                             *
 *                                                                            *
 * Removal or modification of this copyright notice is prohibited.            *
 *                                                                            *
 ******************************************************************************/
//
//  lp_swap.rs
//  seed_metrics
//

use mm2_core::mm_ctx::MmArc;
use std::path::PathBuf;

use super::SavedSwap;

pub fn output_file_path(root: &str, filename: &str) -> PathBuf { std::path::Path::new(root).join(filename) }

pub enum SerializeFormat {
    Json,
    Bytes,
}

/// Persist data to disk, either as JSON or as bytes (via `postcard`).
pub async fn save_to_disk<T: serde::Serialize>(t: T, path: &str, format: &SerializeFormat) -> Result<(), String> {
    let file_extension: &str = match format {
        SerializeFormat::Json => "json",
        SerializeFormat::Bytes => "bytes",
    };

    let path = output_file_path("/home/mike/work/TillyHK/", format!("taker.{}", file_extension).as_str());

    match format {
        SerializeFormat::Json => {
            if let Err(e) = mm2_io::fs::write_json(&t, &path, false).await {
                Err("Error attempting to write JSON to disk".to_string())
            } else {
                Ok(())
            }
        },
        SerializeFormat::Bytes => {
            if let Err(e) = mm2_io::fs::postcard::write_bytes(&t, &path, false).await {
                Err("Error attempting to write bytes to disk".to_string())
            } else {
                Ok(())
            }
        },
    }
}

pub async fn process_seednode_metrics(ctx: &MmArc, swap: &SavedSwap) -> Result<(), String> {
    // Option<(&'static str, OwnedSqlNamedParams)>
    let output_dir = "/home/mike/work/TillyHK/";

    let x = match swap {
        SavedSwap::Taker(taker) => {
            save_to_disk(&taker, output_dir, &SerializeFormat::Bytes).await?;
            ()
        },
        _ => ()
    };

    Ok(())
}
