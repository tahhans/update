#!/usr/bin/env bash
# 生成 RSA-2048 签名密钥对，并导出可内置到 updater 的公钥片段。
#
# 用法:
#   KEY_ID=prod-2026 KEY_DIR=./keys ./scripts/gen-keys.sh
#
# 产物（KEY_DIR 下）:
#   private.pem  私钥，务必离线保管、严禁上传仓库或发布包
#   public.pem   公钥 PEM（备查/分发用）
# 终端会打印一段 Rust 片段，把它填进 src/signature.rs 的 TRUSTED_KEYS。
set -euo pipefail

KEY_ID="${KEY_ID:-prod-2026}"
KEY_DIR="${KEY_DIR:-./keys}"
mkdir -p "$KEY_DIR"

PRIVATE="$KEY_DIR/private.pem"
PUBLIC="$KEY_DIR/public.pem"

if [[ -f "$PRIVATE" ]]; then
  echo "私钥已存在: $PRIVATE（不覆盖）"
else
  openssl genrsa -out "$PRIVATE" 2048
  chmod 600 "$PRIVATE"
fi
openssl rsa -in "$PRIVATE" -pubout -out "$PUBLIC"

python3 - "$PRIVATE" "$KEY_ID" <<'PY'
import base64, binascii, subprocess, sys

private_pem, key_id = sys.argv[1], sys.argv[2]

# 取模数（hex）
mod_hex = subprocess.check_output(
    ["openssl", "rsa", "-in", private_pem, "-modulus", "-noout"],
    text=True,
).strip().split("=", 1)[1]
mod_bytes = binascii.unhexlify(mod_hex)

# 公钥指数固定 65537（010001），big-endian。
exp_bytes = (65537).to_bytes(3, "big")

mod_b64 = base64.b64encode(mod_bytes).decode("ascii")
exp_b64 = base64.b64encode(exp_bytes).decode("ascii")

print()
print("把下面这段填入 src/signature.rs 的 TRUSTED_KEYS：")
print()
print(f'    TrustedKey {{')
print(f'        id: "{key_id}".to_string(),')
print(f'        modulus: "{mod_b64}".to_string(),')
print(f'        exponent: "{exp_b64}".to_string(),')
print(f'    }},')
print()
print("注意：private.pem 只在发布机离线使用；TRUSTED_KEYS 里只放公钥模数/指数，不放私钥。")
PY
