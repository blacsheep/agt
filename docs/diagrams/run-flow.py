from svgkit import Diagram

# Overview: claim, engine choice, the Harness path and the two ways a Run ends.
o = Diagram(720, 10, 'Run 执行总览',
            'RunRequest 先规范化并持久化 claim；新请求按 Agent 执行方式进入 Native 循环或 Harness，'
            '最终保存 Run 终态，或保存检查点后返回 suspended。')
L, C, R = 30, 210, 390   # left, center and right column x
LW, CW, RW = 300, 300, 300
step = 60

y = 16
run = o.node(C, y, CW, 36, 'RunRequest', kind='end'); y += step
claim = o.node(C, y, CW, 36, '规范化参数并持久化 claim'); y += step
key = o.node(C, y, CW, 36, '请求键已存在？', kind='dec'); y += step + 20
same = o.node(L - 10, y, 170, 36, '返回已有记录', kind='end')
engine = o.node(C, y, CW, 36, 'Agent 执行方式？', kind='dec')
reject = o.node(560, y, 140, 36, '拒绝', kind='stop'); y += step + 30
native = o.node(L, y, LW, 64, 'Native 循环', '模型请求、工具批次与能力刷新，见下图')
h1 = o.node(R, y, RW, 48, '获取会话执行锁', '读取配置与线程映射'); y += 72
h2 = o.node(R, y, RW, 48, '启动 Codex app-server', '建立或恢复 thread'); y += 72
h3 = o.node(R, y, RW, 48, '执行 turn', '桥接框架工具、审批和公开进度'); y += 72
h4 = o.node(R, y, RW, 36, '协议确认结果', kind='dec'); y += 36 + 64
wait = o.node(C, y, CW, 36, '保存检查点，返回 suspended', kind='end'); y += 36 + 56
done = o.node(L, y, 660, 36, '保存 Run 终态', kind='end'); y += 36 + 16
o.height = y

cx = C + CW / 2
o.edge((cx, run[1] + 37), (cx, claim[1] - 2))
o.edge((cx, claim[1] + 37), (cx, key[1] - 2))
o.edge((cx, key[1] + 37), (cx, engine[1] - 2), label='新请求', at=(cx + 8, engine[1] - 14))
ky = key[1] + 18
o.edge((C - 1, ky), (same[0] + 85, ky), (same[0] + 85, same[1] - 2), label='参数相同', at=(C - 12, ky - 6), anchor='end')
o.edge((C + CW + 1, ky), (reject[0] + 70, ky), (reject[0] + 70, reject[1] - 2), label='参数冲突', at=(C + CW + 12, ky - 6))
ey = engine[1] + 37
o.edge((C + 70, ey), (C + 70, ey + 20), (L + LW / 2, ey + 20), (L + LW / 2, native[1] - 2),
       label='Model', at=(C + 62, ey + 14), anchor='end')
o.edge((C + CW - 70, ey), (C + CW - 70, ey + 20), (R + RW / 2, ey + 20), (R + RW / 2, h1[1] - 2),
       label='Harness', at=(C + CW - 62, ey + 14))
for a, b in [(h1, h2), (h2, h3), (h3, h4)]:
    o.edge((R + RW / 2, a[1] + a[3] + 1), (R + RW / 2, b[1] - 2))
nb = native[1] + 65
o.edge((270, nb), (270, wait[1] - 2), label='工具要求挂起', at=(262, wait[1] - 40), anchor='end')
o.edge((60, nb), (60, done[1] - 2), label='完成、失败或取消', at=(68, done[1] - 40))
hb = h4[1] + 37
o.edge((450, hb), (450, wait[1] - 2), label='确认安全等待', at=(458, hb + 20))
o.edge((660, hb), (660, done[1] - 2), label='正常完成；失败或\n结果不确定', at=(652, done[1] - 54), anchor='end')

# Native loop: model requests, the tool batch and the capability refresh.
n = Diagram(720, 10, 'Native 执行循环',
            '每轮先检查取消与执行资格，再请求模型；工具调用整批校验后逐项审批执行。'
            '本批次写入自身 MCP/Skill 时在批次结束后刷新能力；成功或失败都回到下一次模型请求。')
A, B, Cc = 36, 290, 520
AW, BW, CW2 = 204, 206, 180
rail = 20
row = lambda i: 16 + i * 74

n1 = n.node(A, row(0), AW, 48, '加载配置快照', '准备上下文、工具和权限')
n2 = n.node(A, row(1), AW, 48, '检查取消与执行资格', '必要时压缩上下文')
n3 = n.node(A, row(2), AW, 48, '请求模型')
fail3 = n.node(B, row(2) + 6, BW, 36, '请求失败或取消 → 终态', kind='stop')
n4 = n.node(A, row(3), AW, 48, '返回类型？', kind='dec')
n5 = n.node(B, row(3), BW, 48, '整批校验 → 逐项权限检查', '审计 → 执行工具 → 保存结果')
fail5 = n.node(Cc, row(3) + 6, CW2, 36, '致命错误或取消 → 终态', kind='stop')
n8 = n.node(A, row(4), AW, 48, '还有可接纳的新输入？', kind='dec')
n6 = n.node(B, row(4), BW, 48, '工具要求挂起？', kind='dec')
wait = n.node(Cc, row(4), CW2, 48, '保存检查点', '返回 suspended', kind='end')
done = n.node(A, row(5), AW, 48, '保存 Run 终态', kind='end')
n7 = n.node(B, row(5), BW, 48, '工具结果加入上下文')
r1 = n.node(B, row(6), BW, 48, '本批次成功写入了自身 MCP / Skill 配置？', kind='dec')
r2 = n.node(Cc, row(6) - 8, CW2, 64, '准备候选能力', '检查模型工具能力、连接、schema 与工具数量')
cancel = n.node(B, row(7) + 6, BW, 36, '取消或失去执行资格 → 终态', kind='stop')
r3 = n.node(Cc, row(7) + 6, CW2, 36, '候选能力就绪？', kind='dec')
r4 = n.node(B, row(8) + 8, BW, 48, '切换能力快照与系统提示词', '更新执行指纹，报告 applied')
r5 = n.node(Cc, row(8) + 8, CW2, 48, '保留旧能力', '配置仍已保存，报告 failed')
bottom = row(8) + 8 + 48 + 22
n.height = int(bottom + 34)

def down(a, b, x=None, label=None):
    x = x if x is not None else a[0] + a[2] / 2
    n.edge((x, a[1] + a[3] + 1), (x, b[1] - 2), label=label, at=(x + 8, b[1] - 10))

def across(a, b, label=None, dashed=False, y=None):
    y = y if y is not None else a[1] + a[3] / 2
    n.edge((a[0] + a[2] + 1, y), (b[0] - 2, y), label=label, at=((a[0] + a[2] + b[0]) / 2, y - 6), anchor='middle', dashed=dashed)

down(n1, n2)
down(n2, n3)
down(n3, n4)
down(n4, n8, label='最终回答')
down(n8, done, label='无')
across(n3, fail3, dashed=True)
across(n4, n5, '工具调用')
across(n5, fail5, dashed=True)
down(n5, n6)
across(n6, wait, '是')
down(n6, n7, label='否')
down(n7, r1)
across(r1, r2, '是', y=r1[1] + 24)
down(r2, r3)
n.edge((Cc - 1, r2[1] + 61), (B + BW / 2, r2[1] + 61), (B + BW / 2, cancel[1] - 2), dashed=True)
n.edge((Cc + CW2 / 2, r3[1] + 37), (Cc + CW2 / 2, r5[1] - 2), label='加载失败', at=(Cc + CW2 / 2 + 8, r5[1] - 10))
n.edge((Cc + 30, r3[1] + 37), (Cc + 30, r3[1] + 50), (B + BW / 2, r3[1] + 50), (B + BW / 2, r4[1] - 2),
       label='是', at=(B + BW / 2 + 8, r4[1] - 10))
# Everything that continues goes back to the eligibility check before the next model request.
for box in (r4, r5):
    n.edge((box[0] + box[2] / 2, box[1] + box[3] + 1), (box[0] + box[2] / 2, bottom), arrow=False, back=True)
n.edge((Cc + CW2 / 2, bottom), (rail, bottom), (rail, n2[1] + 24), (A - 2, n2[1] + 24), back=True)
n.edge((B - 1, r1[1] + 24), (rail, r1[1] + 24), arrow=False, back=True, label='否', at=(B - 14, r1[1] + 18), anchor='end')
n.edge((A - 1, n8[1] + 24), (rail, n8[1] + 24), arrow=False, back=True, label='有', at=(A - 4, n8[1] + 18), anchor='end')
n.text(B + 8, bottom + 18, '回到下一次模型请求之前', 'el bl')

diagrams = {'run-flow': o, 'run-flow-native': n}
