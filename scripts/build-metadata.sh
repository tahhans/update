#!/usr/bin/env bash
# 汇总产物哈希、生成 .sha256 边车文件与 latest-version.json 签名信封。
#
# 用法:
#   PRODUCT=my-shot VERSION=1.5.7 PLATFORM=windows-x64 \
#   ARTIFACT_DIR=./dist OUT_DIR=./dist ./scripts/build-metadata.sh
#
# 带签名（生产）:
#   PRIVATE_KEY_PEM=./keys/private.pem KEY_ID=prod-2026 \
#   PRODUCT=my-shot VERSION=1.5.7 PLATFORM=windows-x64 \
#   ARTIFACT_DIR=./dist ./scripts/build-metadata.sh
#
# 产物:
#   <asset>.sha256               每个安装包/便携包的哈希边车文件
#   latest-version.payload.json  解码后的明文清单（便于人工核对，不发布）
#   latest-version.json          外层信封 {schema,keyId,payload(base64),signature(base64)}
#
# 签名: RSA-PSS + SHA256，salt 长度 32，覆盖**解码后的 payload 明文字节**
#       （与 updater 侧 verify_envelope 保持一致）。无私钥时 signature 留空，
#       开发期可让 updater 走 Release API 源，不强制验签。
set -euo pipefail

PRODUCT="${PRODUCT:?需要 PRODUCT}"
VERSION="${VERSION:?需要 VERSION}"
PLATFORM="${PLATFORM:?需要 PLATFORM}"
ARTIFACT_DIR="${ARTIFACT_DIR:-./dist}"
OUT_DIR="${OUT_DIR:-.}"
KEY_ID="${KEY_ID:-${PRODUCT}-${VERSION%%.*}}"
PRIVATE_KEY_PEM="${PRIVATE_KEY_PEM:-}"

mkdir -p "$OUT_DIR"

python3 - "$ARTIFACT_DIR" "$PRODUCT" "$VERSION" "$PLATFORM" "$OUT_DIR" "$KEY_ID" <<'PY'
import base64, hashlib, json, os, sys, glob
from datetime import datetime, timezone

artifact_dir, product, version, platform, out_dir, key_id = sys.argv[1:7]

# 资产分类规则: 文件名关键字 -> (variant, kind)
def classify(name: str):
    n = name.lower()
    if "portable" in n:
        return "portable", "portable"
    if n.endswith("-demo.zip"):
        return "portable", "portable"
    if "update" in n:
        variant = "offline" if "offline" in n else "online"
        return variant, "update"
    if n.endswith(".exe"):
        variant = "offline" if "offline" in n else "online"
        return variant, "installer"
    return None, None

packages = []
for path in sorted(glob.glob(os.path.join(artifact_dir, "*"))):
    name = os.path.basename(path)
    if name.endswith(".sha256") or name.startswith("latest-version") or name.startswith("."):
        continue
    if not (name.endswith(".exe") or name.endswith(".zip")):
        continue
    variant, kind = classify(name)
    if kind is None:
        continue
    h = hashlib.sha256()
    size = 0
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
            size += len(chunk)
    digest = h.hexdigest()
    packages.append({
        "variant": variant,
        "kind": kind,
        "path": name,
        "size": size,
        "sha256": digest,
    })
    # 写 .sha256 边车文件: `<hash>  <filename>`
    with open(os.path.join(artifact_dir, name + ".sha256"), "w", encoding="utf-8") as f:
        f.write(f"{digest}  {name}\n")

payload = {
    "schema": 1,
    "product": product,
    "version": version,
    "publishedAt": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
    "platform": platform,
    "packages": packages,
}

# 用于签名与信封编码的 payload 明文字节（紧凑序列化，保证两端字节一致）。
payload_bytes = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
payload_b64 = base64.b64encode(payload_bytes).decode("ascii")

os.makedirs(out_dir, exist_ok=True)
# 明文清单（缩进，仅用于人工核对，不上传 Release）。
with open(os.path.join(out_dir, "latest-version.payload.json"), "w", encoding="utf-8") as f:
    json.dump(payload, f, ensure_ascii=False, indent=2)

# 临时原始字节文件，供 openssl 签名；签完即删。
raw_path = os.path.join(out_dir, ".payload.raw")
with open(raw_path, "wb") as f:
    f.write(payload_bytes)

envelope = {
    "schema": 1,
    "keyId": key_id,
    "payload": payload_b64,
    "signature": "",
}
with open(os.path.join(out_dir, "latest-version.json"), "w", encoding="utf-8") as f:
    json.dump(envelope, f, ensure_ascii=False, indent=2)

print(f"packages: {len(packages)} -> {out_dir}/latest-version.json")
PY

# ---- RSA-PSS 签名（salt 32），覆盖 .payload.raw 明文字节 ----
if [[ -n "$PRIVATE_KEY_PEM" && -f "$PRIVATE_KEY_PEM" ]]; then
  echo "[签名] 使用 $PRIVATE_KEY_PEM 做 RSA-PSS-SHA256 签名"
  RAW="$OUT_DIR/.payload.raw"
  SIG_BIN="$OUT_DIR/.payload.sig.bin"
  openssl dgst -sha256 \
    -sigopt rsa_padding_mode:pss \
    -sigopt rsa_pss_saltlen:32 \
    -sign "$PRIVATE_KEY_PEM" \
    -out "$SIG_BIN" "$RAW"
  SIG_B64="$(base64 -w0 "$SIG_BIN")"
  python3 - "$OUT_DIR/latest-version.json" "$SIG_B64" <<'PY'
import json, sys
p, sig = sys.argv[1], sys.argv[2]
with open(p, encoding="utf-8") as f:
    env = json.load(f)
env["signature"] = sig
with open(p, "w", encoding="utf-8") as f:
    json.dump(env, f, ensure_ascii=False, indent=2)
print("[签名] signature 已回填")
PY
  rm -f "$SIG_BIN"
else
  echo "[跳过签名] 未提供 PRIVATE_KEY_PEM，signature 留空（开发期走 API 源）。"
fi

rm -f "$OUT_DIR/.payload.raw"
