#!/usr/bin/env bash
# =============================================================================
# check-real-db-untouched.sh —— 真机账本**带外**不变量检查（2026-10-03 Task 16）
# =============================================================================
#
# 为什么是脚本而不是 `#[test]`：一个 `#[test]` 只能观察**它自己那段窗口**；要发现
# 「**别的**测试碰了真机库」是**运行级**不变量 —— 必须裹在整条命令外面（before → 命令 → after）。
#
# ## 比什么（只有这四张表）
#   usage_detail / usage_session / usage_cursor / usage_daily
#   每表两项：① **行数**；② **按主键 ORDER BY 后的规范化内容哈希**（sha256，
#   列间用 US=0x1f 分隔、NULL 用固定标记，逐行喂给哈希函数）。
#   **四表之外的任何表一律不参与比对**（用量采集只写这四张）。
#
# ## 为什么**不**比对整库 `mam.db` 的 sha256
#   运行中的 MAM app 每 ~60 秒写一次心跳，**整库哈希测的是「app 有没有在跑」而不是
#   「测试有没有碰账本」**，在真机上**必然假红**。实测（闲置实验）：90 秒内无任何测试活动，
#   文件 sha256 变了 2 次；逐行 diff 定位到只有 `remote_devices.last_seen` 与
#   `session_archive.updated_at` 两张**非用量表**在变。
#
# ## 红了先看什么（**第一分辨手段**）
#   本 app 自己就带着用量采集（本分支的功能，`collectIntervalMin` 默认 10 分钟一轮）
#   ⇒ **四表前后不一致不一定是测试碰的，也可能是 app 自己写的**。
#   所以脚本每次取样都会显式报告「**app 是否在跑 + PID**」：
#     * 输出里 `app 状态: 在跑 (PID=…)` ⇒ 先把取样窗口与 app 的采集周期对一下，
#       分辨不了就**如实标为不可判定**，不要猜；
#     * 输出里 `app 状态: 未在跑` ⇒ 四表变化只可能来自被检命令本身。
#
# ## 取样**绝不能**静默失败（Task 16 修复轮 1 / Important，评审实测抓获）
#   本脚本用 `set -uo pipefail` 而**没有** `-e`。漏检返回值时，一次失败的查询会得到**空串**，
#   快照退化成 `usage_detail||<空串的 sha256>`；**两次取样都丢掉同一条查询 ⇒ diff 相等 ⇒
#   脚本印「四表逐字不变」并 exit 0，而实际上什么都没测到**。而那个 exit 0 正是要拿去当证据的
#   东西 ⇒ 这是本脚本**最危险**的失败模式。
#   因此：① 所有查询只走唯一的出口 `q()`，**失败即非零退出**；
#   ② 行数必须是**纯数字**、快照必须**恰好四行**（结构断言）；
#   ③ 每条查询都带 busy timeout —— 本库是 **rollback journal**（仓内没有任何
#      `journal_mode`/`WAL` 设置），读者会与运行中 app 的 ~60 秒心跳争用，
#      默认 timeout=0 会立刻 `database is locked (5)`（评审实测）。
#
# ## 对真机库**只读**（Task 16 修复轮 1）
#   本脚本对真机库的**每一处**打开都是 `sqlite3 -readonly`（含自检里做副本的 `.backup`），
#   且只执行 SELECT / PRAGMA。副本做出来之后，变异只发生在**副本**上。
#
# ## 用法
#   scripts/check-real-db-untouched.sh "<命令>"      # 常规：包裹门禁命令
#   scripts/check-real-db-untouched.sh --self-test    # 自检（见下）
#
# ## 退出码
#   0 = 四表逐字不变（**账本没被碰**；被检命令**自身**的退出码单独打印，不参与本判定）
#   1 = 四表变了（**红**：有东西写了真机账本）
#   2 = 环境/用法错误（无 sqlite3、库不存在、没给命令）
#   3 = 自检失败
#   4 = **取样失败**（fail-closed：非零退出，绝不印「四表逐字不变」）
#
# ## `MAM_CHECK_DB_PATH` 这个覆盖开关为什么存在（**只给自检用**）
#   计划书原文要求自检是「手工往**真机库**塞一行或改一行再跑脚本 → 必须非零退出」。
#   但**写用户的真机账本**恰恰是本任务存在的**唯一**目的所要阻止的事 —— 用真库做自检
#   等于为了证明守卫有效而先违反守卫。因此本脚本接受一个**可选的库路径覆盖**：
#   自检用 `.backup` 复制一份，**改副本**，要求脚本对副本非零退出。
#   覆盖开关**只为这一条自检路径存在**；常规门禁**一律走默认真库**（不设该变量即可），
#   且**默认路径写死在下面**（`$HOME/.mam/mam.db`）。
#
#   库路径有**两条**重定向路径，横幅会**分别打印**（措辞是承重的：自检腿 ④ 靠它证明
#   环境变量覆盖真的生效）：
#     * **内部参数**重定向 —— 自检腿 ①②③ 直接以 argv 传副本路径（`run_check <cmd> <db>`）；
#     * **`MAM_CHECK_DB_PATH` 环境变量覆盖** —— 自检腿 ④ 走**真入口**子进程复验覆盖开关。
# =============================================================================

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# 本脚本自身的绝对路径（自检第 ④ 腿要经**真入口**子进程复验覆盖开关）
SELF_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"

# 真机库默认路径（Task 16：绝不允许被覆盖开关之外的任何东西改写）
REAL_DB_DEFAULT="${HOME}/.mam/mam.db"
DB_PATH="${MAM_CHECK_DB_PATH:-$REAL_DB_DEFAULT}"

# 用量四表（**只有这四张**参与比对）
TABLES=(usage_detail usage_session usage_cursor usage_daily)

US=$'\x1f'            # 列分隔符（Unit Separator）
NULLMARK='<NULL>'    # NULL 占位（argv 里不能传 NUL）

# sqlite3 的 busy timeout（ms）。本库是 rollback journal，读者会与 app 心跳争用。
BUSY_TIMEOUT_MS="${MAM_CHECK_TIMEOUT_MS:-5000}"

# 三条重定向横幅（自检腿 ④ 会 grep 其中的环境变量那条 —— 措辞是承重的，别改）
REDIRECT_NONE="无（默认真机库）"
REDIRECT_INTERNAL="⚠️ 库路径由**内部参数**重定向（自检腿 ①②③ 专用）"
REDIRECT_ENV="⚠️ 库路径被 MAM_CHECK_DB_PATH 环境变量覆盖（自检腿 ④ 专用）"

# 用法/环境错误
die() {
    echo "check-real-db-untouched: $*" >&2
    exit 2
}

# 取样失败：**fail-closed**，非零退出（exit 4），绝不走到「印绿」那一段
sampling_die() {
    echo "check-real-db-untouched: ❌ $*" >&2
    echo "check-real-db-untouched: **fail-closed**：取样失败一律非零退出，**绝不**印「四表逐字不变」（Task 16 修复轮 1）" >&2
    exit 4
}

command -v sqlite3 >/dev/null 2>&1 || die "找不到 sqlite3（本检查的唯一依赖）"
[ -f "$DB_PATH" ] || die "账本不存在：$DB_PATH"

hash_stdin() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 | awk '{print $1}'
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        die "找不到 shasum / sha256sum"
    fi
}

# ---- **唯一的查询出口**（Task 16 修复轮 1 / Important）：失败即返回非零 -------------
# 结果放全局 `Q_OUT`（**刻意不用 `$(q …)`**：`exit` 在命令替换的子 shell 里只退子 shell，
# 拦不住主脚本 —— 那正是假绿的温床）。所有调用点都写 `q … || sampling_die …`。
# `q <库> <SQL> <说明> [tab]`：`tab` = 内容序列化（**必须**带 US 分隔符 + NULL 标记）；
# 省略 = 结构化查询（`PRAGMA` / `COUNT(*)`：它们用**默认**的 `|` 分列，`-separator` 会把
# `PRAGMA table_info` 的 `|` 换成 US，把列序/主键解析直接打瞎 —— 修复轮实测踩过）。
Q_OUT=""
q() {
    local db="$1" sql="$2" what="$3" tabular="${4:-}" err rc
    err="$(mktemp)"
    if [ "$tabular" = "tab" ]; then
        Q_OUT="$(sqlite3 -batch -noheader -readonly -separator "$US" -nullvalue "$NULLMARK" \
            -cmd ".timeout $BUSY_TIMEOUT_MS" "$db" "$sql" 2>"$err")"
    else
        Q_OUT="$(sqlite3 -batch -noheader -readonly -cmd ".timeout $BUSY_TIMEOUT_MS" \
            "$db" "$sql" 2>"$err")"
    fi
    rc=$?
    if [ "$rc" -ne 0 ]; then
        echo "check-real-db-untouched: 取样失败（${what}）：sqlite3 退出码 ${rc} —— $(head -1 "$err")" >&2
        rm -f "$err"
        return 1
    fi
    rm -f "$err"
    return 0
}

# ---- 表结构自省（列序与主键都从库本身读，不手抄）-------------------------------
COLS_OUT=""
all_cols() {
    local db="$1" t="$2"
    q "$db" "PRAGMA table_info(\"$t\");" "表 ${t} 的列序" || return 1
    COLS_OUT="$(printf '%s' "$Q_OUT" | awk -F'|' '{printf "%s\"%s\"", (NR > 1 ? "," : ""), $2}')"
    [ -n "$COLS_OUT" ] || {
        echo "check-real-db-untouched: 表 ${t} 读不到列序（结果为空）" >&2
        return 1
    }
    return 0
}

PK_OUT=""
pk_cols() {
    local db="$1" t="$2"
    q "$db" "PRAGMA table_info(\"$t\");" "表 ${t} 的主键" || return 1
    PK_OUT="$(printf '%s' "$Q_OUT" | awk -F'|' '$6 + 0 > 0 {print $6 "|" $2}' | sort -n |
        cut -d'|' -f2 | awk '{printf "%s\"%s\"", (NR > 1 ? "," : ""), $0}')"
    [ -n "$PK_OUT" ] || {
        echo "check-real-db-untouched: 表 ${t} 没有主键 —— 本脚本的「按主键排序」口径不成立" >&2
        return 1
    }
    return 0
}

# ---- 一个快照：四表 ×（行数 + 主键序规范化内容哈希）----------------------------
# 返回非零 ⇒ 调用方**必须** fail-closed（见 `sampling_die`）。
snapshot() {
    local db="$1" t pk cols count body h
    for t in "${TABLES[@]}"; do
        pk_cols "$db" "$t" || return 1
        pk="$PK_OUT"
        all_cols "$db" "$t" || return 1
        cols="$COLS_OUT"

        q "$db" "SELECT COUNT(*) FROM \"$t\";" "表 ${t} 的行数" || return 1
        count="$Q_OUT"
        # 行数必须是**纯数字**：查询「成功但返回怪东西」也不许静默进快照
        case "$count" in
            '' | *[!0-9]*)
                echo "check-real-db-untouched: 表 ${t} 的行数不是纯数字：『${count}』" >&2
                return 1
                ;;
        esac

        q "$db" "SELECT $cols FROM \"$t\" ORDER BY $pk;" "表 ${t} 的内容" tab || return 1
        body="$Q_OUT"
        # 序列化**必须真的**用 US 分隔符、NULL 必须真的用固定标记（Task 16 修复轮 1）：
        # 我自己在重写 `q()` 时漏掉过 `-separator`，于是「规范化内容哈希」悄悄退化成 sqlite3
        # 默认的 `|` 分隔 + 空串 NULL —— 值里带 `|` 就可能与字段边界撞车、NULL 与空串不可分，
        # 判据被**无声削弱**（两个不同的库可能哈希相同）。这条断言把那类退化变成响亮红。
        if [ "$count" -gt 0 ]; then
            case "$body" in
                *"$US"*) ;;
                *)
                    echo "check-real-db-untouched: 表 ${t} 的内容没有使用 US 分隔符 —— 规范化序列化退化（检查 q() 里的 -separator / -nullvalue 是否还在）" >&2
                    return 1
                    ;;
            esac
        fi

        # 内容哈希（Task 16 修复轮 2）：**先取值、后断言**。
        # 旧写法 `printf … "$(printf … | hash_stdin)"` 里，命令替换的失败状态被 `printf`
        # （内建，恒 0）吞掉 —— `hash_stdin` 里的 `die` 只退子 shell，主脚本照样往下走；
        # 而坏掉/缺失的 `shasum` 会让**两次取样都得到空哈希**、diff 相等 ⇒ 印「四表逐字不变」
        # 并 exit 0（与行数同类的假绿，行数那头有纯数字断言、哈希这头当时没有）。
        # 故：赋值本身判状态（`die` → 取样失败 exit 4 路径），再拒空串 / 非小写十六进制 / 长度≠64。
        h="$(printf '%s\n' "$body" | hash_stdin)" || return 1
        case "$h" in
            '' | *[!0-9a-f]*)
                echo "check-real-db-untouched: 表 ${t} 的内容哈希不是非空的小写十六进制：『${h}』（哈希工具坏了 / 没有输出 —— fail-closed）" >&2
                return 1
                ;;
        esac
        if [ "${#h}" -ne 64 ]; then
            echo "check-real-db-untouched: 表 ${t} 的内容哈希长度不是 64（sha256）：『${h}』（fail-closed）" >&2
            return 1
        fi

        printf '%s|%s|%s\n' "$t" "$count" "$h" || return 1
    done
}

# ---- app 是否在跑（必须显式报告，否则下次假红又要重新花一轮定位）--------------
app_state() {
    local db="$1" pid="" holder="" n=""
    if command -v lsof >/dev/null 2>&1; then
        pid="$(lsof -t "$db" 2>/dev/null | head -1 || true)"
        [ -n "$pid" ] && holder="持有 $db 的进程"
    fi
    if [ -z "$pid" ] && command -v pgrep >/dev/null 2>&1; then
        pid="$(pgrep -f 'MultiAgents|multi-agents' 2>/dev/null | head -1 || true)"
        [ -n "$pid" ] && holder="pgrep 命中 MultiAgents|multi-agents"
    fi
    if [ -n "$pid" ]; then
        n="$(ps -p "$pid" -o command= 2>/dev/null | head -1 || true)"
        echo "app 状态: 在跑 (PID=$pid) [$holder] ${n:-(命令行不可读)}"
        echo "          ⇒ 红时第一分辨手段：app 自己的采集周期（collectIntervalMin 默认 10 分钟）"
    else
        echo "app 状态: 未在跑（四表变化只可能来自被检命令）"
    fi
}

# ---- 核心检查：before → 命令 → after ------------------------------------------
# 两次取样紧贴被检命令，中间不留其它活动。
# `run_check <命令> <库路径> [重定向横幅]`
run_check() {
    local cmd="$1" db="$2" redirect_label="${3:-$REDIRECT_NONE}" verdict cmd_rc=0
    local before_file after_file
    before_file="$(mktemp)"
    after_file="$(mktemp)"

    echo "== 真机账本不变量检查 =="
    echo "库: $db"
    echo "命令: $cmd"
    echo "重定向: ${redirect_label}"

    echo "-- 取样 ① (before) $(date '+%Y-%m-%d %H:%M:%S') --"
    app_state "$db"
    snapshot "$db" >"$before_file" || sampling_die "取样 ①（before）失败"
    # 结构断言：快照必须**恰好四行**（少一行 = 有表被静默跳过 ⇒ 那条表根本没测）
    [ "$(grep -c '^usage_' "$before_file")" = "${#TABLES[@]}" ] ||
        sampling_die "取样 ①（before）拿到的快照不是 ${#TABLES[@]} 行 —— 有表被静默跳过"
    cat "$before_file"

    echo "-- 执行被检命令 $(date '+%Y-%m-%d %H:%M:%S') --"
    (cd "$REPO_ROOT" && bash -c "$cmd")
    cmd_rc=$?
    echo "-- 被检命令退出码: $cmd_rc --"

    echo "-- 取样 ② (after) $(date '+%Y-%m-%d %H:%M:%S') --"
    app_state "$db"
    snapshot "$db" >"$after_file" || sampling_die "取样 ②（after）失败"
    [ "$(grep -c '^usage_' "$after_file")" = "${#TABLES[@]}" ] ||
        sampling_die "取样 ②（after）拿到的快照不是 ${#TABLES[@]} 行 —— 有表被静默跳过"
    cat "$after_file"

    if diff -u "$before_file" "$after_file" >/dev/null; then
        echo "判定: ✅ 四表逐字不变（行数 + 主键序规范化内容哈希全等）"
        verdict=0
    else
        echo "判定: ❌ 四表发生变化 —— 有东西写了真机账本"
        echo "---- 差异（四表，仅此四表）----"
        diff -u "$before_file" "$after_file" || true
        verdict=1
    fi
    rm -f "$before_file" "$after_file"

    if [ "$cmd_rc" -ne 0 ]; then
        echo "注意: ⚠️ 被检命令**自身**退出码 ${cmd_rc}（非 0）。本判定只回答「账本有没有被碰」，"
        echo "      命令自身的红灯请按它自己的门禁口径处置（本仓有已知预期红，见计划 §5）。"
    fi
    return "$verdict"
}

# ---- 自检（锁表第 3 条）：对**副本**做「改内容」与「改行数」两种变异，必须非零退出 ---
self_test() {
    echo "== 自检：脚本自己必须能红（永远不会失败的检查等于没有）=="
    local real_db="$REAL_DB_DEFAULT"
    [ -f "$real_db" ] || die "自检需要真机库存在：$real_db"
    local work
    work="$(mktemp -d)"
    local copy="$work/mam-copy.db"
    # **只读**打开真机库做副本（Task 16 修复轮 1）：本脚本对真机库的每一处打开都必须
    # 是 `-readonly`，否则与文件头的不变量、与报告「零字节写入」的说法自相矛盾——
    # 而四表比对**证明不了**这一点。实测：`-readonly` 下 `.backup` 照常可用（rc 0、副本可读）。
    sqlite3 -readonly "$real_db" ".backup '${copy}'" ||
        die "复制真机库失败（.backup，只读打开）"
    echo "副本: ${copy}（**真机库只被只读打开了这一次**，全程无一字节写入）"

    # 四表皆空就没法做变异自检 —— **响亮失败**，不静默跳过
    q "$copy" "SELECT (SELECT COUNT(*) FROM usage_detail)+(SELECT COUNT(*) FROM usage_session)+(SELECT COUNT(*) FROM usage_cursor)+(SELECT COUNT(*) FROM usage_daily);" \
        "自检：四表总行数" || sampling_die "自检前置查询失败"
    local total="$Q_OUT"
    case "$total" in
        '' | *[!0-9]*) sampling_die "自检：四表总行数不是纯数字：『${total}』" ;;
    esac
    [ "$total" -gt 0 ] || {
        echo "自检失败: 四表全空，无法做变异自检（不静默跳过）" >&2
        rm -rf "$work"
        exit 3
    }

    local rc=0

    # ① 干净副本 + 无副作用命令 → 必须 exit 0（证明脚本不是「恒红」）
    echo "-- 自检 ① 干净副本 → 期望 exit 0 --"
    if run_check "true" "$copy" "$REDIRECT_INTERNAL" >"$work/selftest-ok.log" 2>&1; then
        echo "自检 ① 通过（exit 0）"
    else
        echo "自检 ① **失败**：干净副本上就红了 —— 检查本身坏了"
        echo "          （本腿走的是副本，app 不可能写它 ⇒ 只可能是脚本坏了）"
        sed -n '1,40p' "$work/selftest-ok.log"
        rc=1
    fi

    # ② 变异「只改内容、不动行数」，且变异**发生在被检命令的窗口内** → 必须非零。
    #    为什么变异必须进窗口：本脚本的判据是「before → 命令 → after」的**运行级**不变量；
    #    在窗口外先改库再跑，两边取样看到的是**同一个**已改状态 ⇒ 必然相等，那是**假绿**。
    #    （这也正是计划书那句自检的真正含义：要造出「被检命令写了账本」这个场景。）
    echo "-- 自检 ② 命令窗口内只改内容（UPDATE 一行，行数不变）→ 期望非零 --"
    q "$copy" "SELECT COALESCE(SUM(updated_at),0) FROM usage_daily;" "自检②：变异前 updated_at 合计" ||
        sampling_die "自检②前置查询失败"
    local sum_before="$Q_OUT"
    local mut_content="sqlite3 \"$copy\" 'UPDATE usage_daily SET updated_at = updated_at + 1 WHERE rowid = (SELECT rowid FROM usage_daily ORDER BY day_key, source_id, project_key, provider, model LIMIT 1);'"
    if run_check "$mut_content" "$copy" "$REDIRECT_INTERNAL" >"$work/selftest-content.log" 2>&1; then
        echo "自检 ② **失败**：窗口内只改内容的变异**没有**让脚本变红 —— 内容哈希那一半是死的"
        rc=1
    else
        q "$copy" "SELECT COALESCE(SUM(updated_at),0) FROM usage_daily;" "自检②：变异后 updated_at 合计" ||
            sampling_die "自检②复核查询失败"
        local sum_after="$Q_OUT"
        if [ "$sum_before" = "$sum_after" ]; then
            echo "自检 ② **失败**：变异根本没生效（updated_at 合计 ${sum_before} 未变）—— 本腿是假绿"
            rc=1
        else
            echo "自检 ② 通过（非零退出；变异已生效：updated_at 合计 ${sum_before} → ${sum_after}）"
        fi
    fi

    # ③ 变异「改行数」（窗口内删一行）→ 必须非零（这一半与内容哈希是两条独立判据）
    echo "-- 自检 ③ 命令窗口内删一行（改行数）→ 期望非零 --"
    q "$copy" "SELECT COUNT(*) FROM usage_daily;" "自检③：变异前行数" || sampling_die "自检③前置查询失败"
    local n_before="$Q_OUT"
    local mut_rows="sqlite3 \"$copy\" 'DELETE FROM usage_daily WHERE rowid = (SELECT rowid FROM usage_daily LIMIT 1);'"
    if run_check "$mut_rows" "$copy" "$REDIRECT_INTERNAL" >"$work/selftest-count.log" 2>&1; then
        echo "自检 ③ **失败**：窗口内删行的变异**没有**让脚本变红 —— 行数那一半是死的"
        rc=1
    else
        q "$copy" "SELECT COUNT(*) FROM usage_daily;" "自检③：变异后行数" || sampling_die "自检③复核查询失败"
        local n_after="$Q_OUT"
        if [ "$n_before" = "$n_after" ]; then
            echo "自检 ③ **失败**：变异根本没生效（usage_daily 行数 ${n_before} 未变）—— 本腿是假绿"
            rc=1
        else
            echo "自检 ③ 通过（非零退出；变异已生效：usage_daily 行数 ${n_before} → ${n_after}）"
        fi
    fi

    # ④ `MAM_CHECK_DB_PATH` 覆盖开关**必须真的生效**（本脚本用 argv 传库路径，但门禁与
    #    自检都可能走 env 覆盖；覆盖坏了 = 自检自己在测另一条路 ⇒ 假绿）。这一腿走
    #    **真入口**（子进程 + argv 校验 + 退出码 + 输出里必须出现**环境变量覆盖**那条横幅，
    #    注意与腿 ①②③ 的「内部参数重定向」横幅**措辞不同** —— 判据靠的就是这个区别）。
    echo "-- 自检 ④ 经 MAM_CHECK_DB_PATH 覆盖 + 真入口（子进程）→ 期望非零且提示覆盖 --"
    local mut2="sqlite3 \"$copy\" 'UPDATE usage_cursor SET byte_offset = byte_offset + 1 WHERE rowid = (SELECT rowid FROM usage_cursor LIMIT 1);'"
    if MAM_CHECK_DB_PATH="$copy" "$SELF_PATH" "$mut2" >"$work/selftest-override.log" 2>&1; then
        echo "自检 ④ **失败**：覆盖路径下的窗口内变异**没有**让脚本非零退出"
        rc=1
    elif ! grep -q 'MAM_CHECK_DB_PATH 环境变量覆盖' "$work/selftest-override.log"; then
        echo "自检 ④ **失败**：退出码对，但输出里没有**环境变量覆盖**横幅 —— 说明它根本没走覆盖路径"
        rc=1
    else
        echo "自检 ④ 通过（覆盖生效、非零退出、横幅已打印）"
    fi

    rm -rf "$work"
    if [ "$rc" -eq 0 ]; then
        echo "自检结论: ✅ 全部通过（脚本既不恒红、也不恒绿）"
        exit 0
    fi
    echo "自检结论: ❌ 失败" >&2
    exit 3
}

# ---- 入口 ---------------------------------------------------------------------
if [ "${1:-}" = "--self-test" ]; then
    self_test
fi

[ $# -ge 1 ] || die "用法: $0 \"<命令>\"  或  $0 --self-test"

# 真入口重定向横幅：只有 `MAM_CHECK_DB_PATH` 这一条路径会走到这里（argv 那条只出现在自检内部）
if [ "$DB_PATH" = "$REAL_DB_DEFAULT" ]; then
    RUN_LABEL="$REDIRECT_NONE"
else
    RUN_LABEL="$REDIRECT_ENV"
fi
run_check "$1" "$DB_PATH" "$RUN_LABEL"
exit $?
