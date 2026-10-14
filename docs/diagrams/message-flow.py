from svgkit import Diagram

d = Diagram(720, 1000, '消息调度流程',
            '用户消息经 Service 校验后由 IM 保存；授权自动回复时建立 Discussion 并唤醒 Scheduler，'
            '准入和参与判断通过后领取执行槽，Runtime 执行后结算并幂等发布回复。')
MX, MW = 30, 330   # main column
RX, RW = 430, 260  # outcomes column
cx, rcx = MX + MW / 2, RX + RW / 2
GAP = 22

y = 16
def main(h, title, sub=None, kind='node'):
    global y
    box = (MX, y, MW, h)
    d.node(MX, y, MW, h, title, sub, kind)
    y += h + GAP
    return box

def down(a, b, label=None):
    d.edge((cx, a[1] + a[3] + 1), (cx, b[1] - 2), label=label, at=(cx + 8, b[1] - 9))

def right(a, b, label):
    yy = a[1] + a[3] / 2
    d.edge((MX + MW + 1, yy), (RX - 2, yy), label=label, at=((MX + MW + RX) / 2, yy - 6), anchor='middle')

A = main(36, '用户发送消息', kind='end')
B = main(36, 'Service 校验身份与会话权限')
C = main(36, 'IM 持久化消息与接收者投递')
D = main(36, '是否授权自动回复？', kind='dec')
F = main(48, '创建或更新 Discussion', '授权具体消息')
G = main(36, '唤醒 Scheduler')
H = main(36, '扫描投递，建立独立调度任务')
I = main(48, '准入条件满足？', '身份、成员、策略、额度、租约', kind='dec')
K = main(36, '需要参与判断？', kind='dec')
y += 20
N = main(48, '进入 ready 队列')
O = main(36, '重新检查资格并领取执行槽')
P = main(36, '组装上下文和 RunRequest')
Q = main(36, 'Runtime 执行 Native 或 Harness')
R = main(36, '保存 Run 结果并结算调度任务')
S = main(48, '复查发布权限与任务版本', '有可发布回复时幂等写入 IM')
T = main(36, '变化 outbox → 事件投影 → SSE', kind='end')

E = d.node(RX, D[1], RW, 36, '仅保存消息', kind='stop')
J = d.node(RX, I[1], RW, 48, '按原因等待、阻塞或终结', kind='stop')
L = d.node(RX, N[1], RW, 48, '参与判断', 'participate / silent / wait', kind='dec')
M = d.node(RX, N[1] + 48 + 36, RW, 36, '记录沉默决定', kind='stop')

for a, b, label in [(A, B, None), (B, C, None), (C, D, None), (D, F, '是'), (F, G, None), (G, H, None),
                    (H, I, None), (I, K, '是'), (K, N, '直接执行路径'), (N, O, None), (O, P, None),
                    (P, Q, None), (Q, R, None), (R, S, None), (S, T, None)]:
    down(a, b, label)
right(D, E, '否')
right(I, J, '否')

# Native peers that were not mentioned decide whether to take part first.
ky = K[1] + K[3] / 2
d.edge((MX + MW + 1, ky), (rcx, ky), (rcx, L[1] - 2), label='Native 未被点名的同伴消息', at=(MX + MW + 12, ky - 6))
ly = L[1] + L[3] / 2
d.edge((RX - 1, ly), (MX + MW + 2, ly), label='participate', at=((MX + MW + RX) / 2, ly - 6), anchor='middle')
d.edge((rcx, L[1] + L[3] + 1), (rcx, M[1] - 2), label='silent', at=(rcx + 8, M[1] - 16))
d.edge((RX + RW + 1, ly), (RX + RW + 16, ly), (RX + RW + 16, J[1] + J[3] / 2), (RX + RW + 2, J[1] + J[3] / 2),
       label='wait', at=(RX + RW + 12, ly - (L[3] / 2) - 22), anchor='end')

d.height = int(y - GAP + 16)
