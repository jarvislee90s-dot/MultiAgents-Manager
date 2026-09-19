# verify-oc.py — opencode 会话 SQLite 对账（读副本，不动活库）
# 用法：python verify-oc.py <stamp>
import sqlite3, os, sys, json, shutil

stamp = sys.argv[1]
src = os.path.expanduser(r'~\.local\share\opencode')
tmp = os.path.expanduser(r'~\probe-opencode\evidence\dbcopy')
os.makedirs(tmp, exist_ok=True)
for f in ('opencode.db', 'opencode.db-wal', 'opencode.db-shm'):
    s = os.path.join(src, f)
    if os.path.exists(s):
        shutil.copy2(s, os.path.join(tmp, f))
db = sqlite3.connect(os.path.join(tmp, 'opencode.db'))
hit = False
for table in ('part', 'message'):
    rows = db.execute(f"select session_id, data from {table} where data like ?", ('%' + stamp + '%',)).fetchall()
    for sid, data in rows[:3]:
        hit = True
        try:
            j = json.loads(data)
            text = j.get('text', '')
            print(f"HIT table={table} session={sid} type={j.get('type')} TEXTLEN={len(text)}")
            print("  CTX:", text[:100].replace('\n', '\\n'))
        except Exception as e:
            print(f"HIT table={table} session={sid} (json parse fail: {e})")
if not hit:
    print("MISS (no part/message row contains stamp)")
