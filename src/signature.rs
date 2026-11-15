//! 发布元数据信封验签（RSA-PSS + SHA256）。
//!
//! 信封结构（对标 Snow Shot）：
//! ```json
//! {
//!   "schema": 1,
//!   "keyId": "prod-2026",
//!   "payload": "<base64(明文 payload JSON)>",
//!   "signature": "<base64(RSA-PSS 签名)>"
//! }
//! ```
//! 签名覆盖的是**解码后的 payload 明文字节**：先对明文做 SHA256 摘要，
//! 再用内置 RSA 公钥按 PSS（salt 长度 32）验签。验签通过后才解析 payload。

use base64::Engine;
use rsa::pss::Pss;
use rsa::{BigUint, RsaPublicKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::AppError;

/// 内置可信公钥：`keyId` 区分多把轮换密钥。
/// `modulus` / `exponent` 均为 big-endian 字节的 base64。
#[derive(Debug, Deserialize, Clone)]
pub struct TrustedKey {
    pub id: String,
    pub modulus: String,
    pub exponent: String,
}

/// 编译期内置的可信公钥列表。
///
/// 生产版由发布脚本导出公钥后填入；当前为空表示尚未内置正式公钥，
/// 此时若强制走信封源会直接拒绝（开发期可先用 Release API 源）。
const TRUSTED_KEYS: &[TrustedKey] = &[];

#[derive(Debug, Deserialize)]
struct Envelope {
    schema: u64,
    #[serde(rename = "keyId")]
    key_id: String,
    payload: String,
    signature: String,
}

fn b64decode(value: &str, what: &str) -> Result<Vec<u8>, AppError> {
    base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .map_err(|_| AppError::Verify(format!("{what} base64 解码失败")))
}

/// 校验信封签名，通过后返回解码并解析出的 payload JSON。
///
/// 任何一步失败（结构、schema、keyId 不受信、签名不符、payload 非法）
/// 都返回 [`AppError::Verify`]，调用方必须拒绝本次更新。
pub fn verify_envelope(envelope: &[u8]) -> Result<serde_json::Value, AppError> {
    let env: Envelope = serde_json::from_slice(envelope)
        .map_err(|_| AppError::Verify("更新信封 JSON 解析失败".to_string()))?;

    if env.schema != 1 {
        return Err(AppError::Verify(format!(
            "不支持的信封 schema: {}",
            env.schema
        )));
    }

    let payload = b64decode(&env.payload, "payload")?;
    let signature = b64decode(&env.signature, "signature")?;

    let key = TRUSTED_KEYS
        .iter()
        .find(|k| k.id == env.key_id)
        .ok_or_else(|| {
            AppError::Verify(format!("keyId `{}` 不在可信公钥列表中", env.key_id))
        })?;

    let modulus = b64decode(&key.modulus, "公钥模数")?;
    let exponent = b64decode(&key.exponent, "公钥指数")?;
    let public_key = RsaPublicKey::new(
        BigUint::from_bytes_be(&modulus),
        BigUint::from_bytes_be(&exponent),
    )
    .map_err(|_| AppError::Verify("内置公钥构造失败".to_string()))?;

    // 签名覆盖 payload 明文摘要，而非 base64 文本。
    let digest = Sha256::digest(&payload);
    public_key
        .verify(Pss::new_with_salt::<Sha256>(32), &digest, &signature)
        .map_err(|_| AppError::Verify("信封签名校验失败，拒绝本次更新".to_string()))?;

    let signed: serde_json::Value = serde_json::from_slice(&payload)
        .map_err(|_| AppError::Verify("签名通过但 payload JSON 非法".to_string()))?;

    Ok(signed)
}

/// 校验 payload 中的产品标识与本软件一致，避免串包。
pub fn expect_product(signed: &serde_json::Value, product: &str) -> Result<(), AppError> {
    let actual = signed
        .get("product")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !product.is_empty() && actual != product {
        return Err(AppError::Verify(format!(
            "产品标识不匹配：信封 `{actual}`，本软件 `{product}`"
        )));
    }
    Ok(())
}
