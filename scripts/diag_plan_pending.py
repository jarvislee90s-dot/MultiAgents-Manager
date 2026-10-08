import sys, json

BAD = set(' \t\n\r"\'<>|' + chr(92))

def extract_plan_file_ref(text):
    marker = 'plans' + chr(92) if False else 'plans'
    i = 0
    while i < len(text):
        a = text.find('plans/', i)
        b = text.find('plans' + chr(92), i)
        cands = [x for x in (a, b) if x != -1]
        if not cands:
            return None
        at = min(cands)
        start = at + 6
        rest = text[start:]
        e = rest.find('.md')
        if e != -1:
            name = rest[:e]
            if name and not (set(name) & BAD):
                return text[at:start + e + 3]
        i = start
    return None

def block_text(c):
    if isinstance(c, str):
        return c
    if isinstance(c, list):
        return ' '.join(
            b.get('text', '') for b in c if isinstance(b, dict) and b.get('type') == 'text'
        )
    return ''

msgs = []
with open(sys.argv[1], encoding='utf-8') as f:
    for line in f:
        try:
            v = json.loads(line)
        except Exception:
            continue
        if v.get('isMeta') is True:
            continue
        t = v.get('type', '')
        content = (v.get('message') or {}).get('content')
        if content is None:
            continue
        if t == 'user':
            if isinstance(content, str):
                if content.strip():
                    msgs.append(('user', content.strip()))
            elif isinstance(content, list):
                for b in content:
                    bt = b.get('type')
                    if bt == 'text':
                        s = b.get('text', '')
                        if s.strip():
                            msgs.append(('user', s))
                    elif bt == 'tool_result':
                        txt = block_text(b.get('content')) or '工具结果'
                        msgs.append(('tool-result', txt))
                        p = extract_plan_file_ref(txt)
                        if p:
                            msgs.append(('plan-file', p))
        elif t == 'assistant':
            if not isinstance(content, list):
                continue
            for b in content:
                bt = b.get('type')
                if bt == 'text':
                    s = b.get('text', '')
                    if s.strip():
                        msgs.append(('assistant', s))
                elif bt == 'thinking':
                    s = b.get('thinking', '')
                    if s.strip():
                        msgs.append(('thinking', s))
                elif bt == 'tool_use':
                    args = b.get('input')
                    plan = args.get('plan') if isinstance(args, dict) else None
                    if isinstance(plan, str) and plan.strip():
                        msgs.append(('plan', plan))
                    else:
                        msgs.append(('tool-call', b.get('name', '')))

print('total msgs:', len(msgs))
last = -1
for i in range(len(msgs) - 1, -1, -1):
    if msgs[i][0] in ('plan', 'plan-file'):
        last = i
        break
print('last plan-class idx:', last, msgs[last][0] if last >= 0 else None,
      str(msgs[last][1])[:70] if last >= 0 else '')
pend = True
for i in range(last + 1, len(msgs)):
    if msgs[i][0] in ('user', 'tool-call', 'tool-result'):
        print('first clearing row at', i, msgs[i][0], str(msgs[i][1])[:70])
        pend = False
        break
print('isPlanPending =', pend)
print('tail kinds:', [k for k, _ in msgs[-12:]])
