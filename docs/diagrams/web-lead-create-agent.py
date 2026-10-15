from svgkit import Diagram, Sequence

# Where the request travels: message path on the left, execution in the middle,
# model and activation on the right. Results return along the outer rails.
f = Diagram(720, 10, 'lead 创建 Agent 的调用路径',
            'Web 保存消息并启动调度，Scheduler 以 lead 的身份执行 NativeRun；framework_resource_write 经校验与审批进入 ToolService，'
            '由 Resources 写入 agent.yaml，工具结果返回 Run。要求成为可用队友时再创建身份并保存绑定；最终回答幂等发布回 IM。')
L, M, R = 24, 260, 496
W = 200
cl, cm, cr = L + W / 2, M + W / 2, R + W / 2

user = f.node(M, 16, W, 36, '用户：请创建 reviewer', kind='end')
web = f.node(M, 76, W, 48, 'Web 控制台', '发送消息并授权自动回复')
refresh = f.node(L, 76, W, 48, 'outbox → events watch', 'SSE → 页面刷新')
im = f.node(L, 168, W, 48, 'agt-service → agt-im', '保存消息与投递')
sched = f.node(R, 168, W, 48, 'Scheduler', '授权、准入、领取 lead 任务')
config = f.node(L, 268, W, 64, 'lead 配置', 'builtin_tools: resource_management')
run = f.node(M, 268, W, 52, 'Runtime / NativeRun', '以 lead 的身份执行')
model = f.node(R, 268, W, 40, 'lead 使用的模型', kind='ext')
publish = f.node(L, 392, W, 48, '调度结算与幂等发布')
gate = f.node(M, 392, W, 48, '参数校验、审批、调度围栏')
activate = f.node(R, 392, W, 60, '启用为可用队友', 'principal_create → agent_bind → binding_list 核验')
service = f.node(M, 476, W, 48, 'ToolService', '固定 lead Actor + 能力校验')
identity = f.node(L, 564, W, 48, 'im/im.db', 'Agent 身份')
resources = f.node(M, 564, W, 48, 'Resources::write_with', '资源锁、领域写入 → AgentStore')
bindings = f.node(R, 564, W, 48, 'runtime/runtime.db', '托管绑定 bindings')
file = f.node(M - 30, 640, W + 60, 36, '', 'agents/default/reviewer/agent.yaml', kind='ext')
result = f.node(M - 30, 700, W + 60, 48, '工具结果', '配置摘要、revision 与 activation', kind='end')
f.height = 764

f.edge((cm, 53), (cm, 74))
f.edge((cl, 166), (cl, 126))
f.edge((M + 40, 125), (M + 40, 180), (L + W + 2, 180), label='Rust CLI', at=(M + 48, 150))
f.edge((M + W - 40, 125), (M + W - 40, 180), (R - 2, 180), label='schedule start\n--message', at=(M + W - 32, 142))
f.edge((L + W + 1, 204), (R - 2, 204), label='待处理投递', at=(cm, 198), anchor='middle')
f.edge((cr, 217), (cr, 244), (M + W - 50, 244), (M + W - 50, 266))
f.edge((L + W + 1, 294), (M - 2, 294), label='挂载', at=((L + W + M) / 2, 288), anchor='middle')
f.edge((M + W + 1, 288), (R - 2, 288), both=True)
f.edge((M + 30, 321), (M + 30, 352), (cl, 352), (cl, 390), label='形成最终回答', at=((cl + M + 30) / 2, 346), anchor='middle')
f.edge((cm, 321), (cm, 390), label='framework_resource_write', at=(cm + 8, 376))
f.edge((M + W - 30, 321), (M + W - 30, 352), (cr, 352), (cr, 390), dashed=True,
       label='若要求成为可用队友', at=((M + W - 30 + cr) / 2, 346), anchor='middle')
f.edge((cm, 441), (cm, 474))
f.edge((cr, 453), (cr, 492), (M + W + 2, 492))
f.edge((M - 1, 500), (cl, 500), (cl, 562), label='创建 Agent 身份', at=((cl + M) / 2, 494), anchor='middle')
f.edge((M + W + 1, 512), (R + 150, 512), (R + 150, 562), label='保存托管绑定', at=((M + W + R + 150) / 2, 506), anchor='middle')
f.edge((cm, 525), (cm, 562))
f.edge((cm, 613), (cm, 638))
f.edge((cm, 677), (cm, 698))
# Returns: the tool result goes back to the Run, the published reply back to IM.
f.edge((M + W + 31, 724), (710, 724), (710, 314), (M + W + 2, 314), back=True)
f.edge((L - 1, 416), (10, 416), (10, 204), (L - 2, 204), back=True)

# Full sequence. The user and the Web console share a column, and the stores
# are folded into ToolService / Resources, so six columns fit the page.
s = Sequence('lead 创建 Agent 的完整时序',
             '消息保存后启动调度，lead 的 NativeRun 请求模型并执行 framework_resource_write；审批与能力检查通过时写入 agent.yaml，'
             '要求成为可用队友时继续创建身份、绑定并核验；最后保存 Run 结果并幂等发布回复。',
             [('W', '用户 / Web', 'agt web'), ('S', 'Service / IM', 'Rust'), ('Q', 'Scheduler', None),
              ('R', 'lead', 'NativeRun'), ('M', '模型', None), ('T', 'ToolService', 'Resources 与存储')], head_h=66
             ).build([
    ('self', 'W', '用户发送创建 reviewer 的消息'),
    ('msg', 'W', 'S', 'CLI tool call message_send'),
    ('reply', 'S', 'W', '已保存消息及 message ID'),
    ('msg', 'W', 'Q', 'schedule start --message ID'),
    ('self', 'Q', '扫描投递、校验授权、领取任务'),
    ('msg', 'Q', 'R', 'RunRequest，trigger principal = lead'),
    ('self', 'R', '加载配置、固定 Actor、挂载资源工具'),
    ('msg', 'R', 'M', '消息上下文与 framework 工具 schema'),
    ('reply', 'M', 'R', 'framework_resource_write 调用'),
    ('self', 'R', '校验参数、PermissionLoop、审计与围栏'),
    ('block', 'alt', [
        ('审批允许且 Service 能力检查通过', [
            ('msg', 'R', 'T', 'resource_write(kind=agent, create=true)'),
            ('self', 'T', '资源锁内校验配置与引用\nAgentStore::create_in 原子写入 agent.yaml'),
            ('reply', 'T', 'R', '摘要 + revision + activation + next_steps'),
            ('msg', 'R', 'M', '工具结果；仅保存定义时尚未启用'),
            ('block', 'opt', [('用户要求成为可用队友，模型继续完成启用', [
                ('block', 'loop', [('按需查询、principal_create、agent_bind、binding_list', [
                    ('reply', 'M', 'R', '下一项 framework 工具调用'),
                    ('self', 'R', '校验、审批、审计与围栏'),
                    ('msg', 'R', 'T', '调用身份或绑定工具'),
                    ('self', 'T', '读取状态、创建身份\n或保存绑定'),
                    ('reply', 'T', 'R', '实际存储结果'),
                    ('msg', 'R', 'M', '结果进入上下文，继续决策'),
                ])]),
            ])]),
            ('reply', 'M', 'R', '根据实际结果生成回答'),
        ]),
        ('审批拒绝或工具业务错误', [
            ('msg', 'R', 'M', '返回拒绝或错误，不能报告创建成功'),
            ('reply', 'M', 'R', '修正参数继续，或说明失败'),
        ]),
    ]),
    ('reply', 'R', 'Q', '保存并返回 Run 结果'),
    ('msg', 'Q', 'S', '复查围栏并幂等发布回复'),
    ('reply', 'S', 'W', '变化经事件投影与 SSE 触发刷新'),
    ('self', 'W', '向用户显示 lead 的回复'),
])

diagrams = {'web-lead-create-agent': f, 'web-lead-create-agent-sequence': s}
