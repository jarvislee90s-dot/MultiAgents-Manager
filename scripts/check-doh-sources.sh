#!/usr/bin/env bash
# check-doh-sources.sh — Tailscale 可达性校验的 DoH 源表**逐源连通性冒烟**（打一次，非单测）
#
# 用法：
#   ./scripts/check-doh-sources.sh                   # 只实测 reach.rs 的 DOH_SOURCES
#   ./scripts/check-doh-sources.sh 120.53.53.53 ::  # 追加候选源一并实测（取舍取证用，见下）
#
# 退出码：0 = 表里每个源都实测可用；1 = 有源不可用（表里出现纸面源，或本网络变了）
#
# 为什么是脚本而不是单测（I-2，评审 2026-10-07）：项目「单测零网络」是红线；而源表里
# 每个 IP 的可达性是**环境事实**，只能实测。旧表的次源 `1.1.1.1` 自陈「本网络未实测」，
# 复测 `nc -z 1.1.1.1 443` 不通 —— 冗余只是名义上的，A3 要修的失败模式原样存在。
# 故：① reach.rs 的源表注释里每个源都要有带日期的实测记录（源码锁测试把关）；
#     ② 换网络/换运营商后跑本脚本复测；③ 新增源前先用本脚本打一次（追加参数形态）。
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REACH_RS="$PROJECT_DIR/src-tauri/src/remote/tailscale/reach.rs"

if [[ ! -f "$REACH_RS" ]]; then
  echo "找不到源表文件: $REACH_RS" >&2
  exit 1
fi

# 源表 = DOH_SOURCES 常量块里的 https 字面量（保持「只认表里写的东西」，不复制一份到脚本）
# 注：不用 mapfile —— macOS 自带 bash 3.2 没有它（本脚本要能在开发机直接跑）
TABLE_SOURCES=()
while IFS= read -r line; do
  [[ -n "$line" ]] && TABLE_SOURCES+=("$line")
done < <(awk '/const DOH_SOURCES/,/^\];/' "$REACH_RS" | grep -oE 'https://[0-9A-Za-z.:]+/[A-Za-z-]+')

if [[ ${#TABLE_SOURCES[@]} -eq 0 ]]; then
  echo "未能从 reach.rs 解析出任何 DoH 源（源表格式变了？）" >&2
  exit 1
fi

EXTRA_SOURCES=()
for arg in "$@"; do
  case "$arg" in
    http*) EXTRA_SOURCES+=("$arg") ;;
    "" | "::") ;;                                    # 纯占位符，忽略
    *) EXTRA_SOURCES+=("https://$arg/resolve") ;;    # 裸 IP/host → 默认 /resolve 形态
  esac
done

probe() {
  local url="$1" qtype="$2" body code
  body="$(mktemp)"
  code="$(curl -s -o "$body" -w '%{http_code}' --max-time 8 "${url}?name=example.com&type=${qtype}" 2>/dev/null)"
  # 判据与生产解析层同源：HTTP 200 ∧ 应答是 DoH JSON（带 Status 字段）——
  # 「连得上」不等于「答得出」，门户页/拦截页也会给 200
  if [[ "$code" == "200" ]] && grep -q '"Status"' "$body"; then
    local n
    n="$(grep -o '"data"' "$body" | wc -l | tr -d ' ')"
    printf '  ✓ %-40s %s %s（Answer 条目 %s）\n' "$url" "$qtype" "$code" "$n"
    rm -f "$body"
    return 0
  fi
  printf '  ✗ %-40s %s %s（无 DoH JSON 应答）\n' "$url" "$qtype" "${code:-000}"
  rm -f "$body"
  return 1
}

echo "== 源表（reach.rs DOH_SOURCES）逐源冒烟：$(date '+%Y-%m-%d %H:%M')"
failed=0
for src in "${TABLE_SOURCES[@]}"; do
  probe "$src" A || failed=1
  probe "$src" AAAA || failed=1
done

if [[ ${#EXTRA_SOURCES[@]} -gt 0 ]]; then
  echo "== 追加候选源（不参与退出码）"
  for src in "${EXTRA_SOURCES[@]}"; do
    probe "$src" A || true
  done
fi

if [[ $failed -ne 0 ]]; then
  echo "结果：**有源不可用** —— 表里存在纸面源（或本网络已变）：请改 reach.rs 的 DOH_SOURCES" >&2
  echo "      并同步其注释里的实测记录 + 源码锁测试 doh_source_table_carries_measured_evidence_and_no_paper_source。" >&2
  exit 1
fi
echo "结果：表里每个源均实测可用（另请顺手更新 reach.rs 注释里的实测日期与记录）。"
