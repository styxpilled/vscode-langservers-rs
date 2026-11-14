import json, sys, collections
for path in sys.argv[1:]:
    pending = {}; out_pending = {}; lat = collections.defaultdict(list); errors = []; cancels = set()
    for line in open(path, encoding='utf-8'):
        m = json.loads(line); msg = m.get('msg'); t = m['t']
        if not isinstance(msg, dict): continue
        d = m['dir']; mid = msg.get('id'); method = msg.get('method')
        if d == 'in' and method == '$/cancelRequest': cancels.add(json.dumps(msg['params']['id']))
        elif d == 'in' and method and mid is not None: pending[json.dumps(mid)] = (method, t)
        elif d == 'out' and method and mid is not None: out_pending[json.dumps(mid)] = (method, t, msg.get('params'))
        elif d == 'out' and method is None:
            k = json.dumps(mid)
            if k in pending:
                meth, t0 = pending.pop(k); lat[meth].append(t - t0)
                if 'error' in msg: errors.append((meth, msg['error']))
        elif d == 'in' and method is None:
            k = json.dumps(mid)
            if k in out_pending:
                meth, t0, p = out_pending.pop(k); lat['(client) ' + meth].append(t - t0)
                if 'error' in msg: errors.append(('(client) ' + meth + ' ' + json.dumps(p), msg['error']))
    print('==', path)
    for meth, l in sorted(lat.items()):
        l.sort(); print(f'  {meth:40} n={len(l):4} median={l[len(l)//2]:6} max={l[-1]:6}')
    for e in errors[:10]: print('  ERROR', e)
    print('  errors', len(errors), 'cancelled ids', len(cancels))
    for k, (meth, t0) in pending.items(): print('  UNANSWERED', k, meth, 'at', t0)
    for k, (meth, t0, p) in out_pending.items(): print('  UNANSWERED BY CLIENT', meth, p)
