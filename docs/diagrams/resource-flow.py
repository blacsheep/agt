from svgkit import Diagram, Sequence

# Entry points into the shared resource service, and the write path.
e = Diagram(720, 10, '共享资源管理入口',
            'CLI 资源命令直接调用 Resources；Web 请求和模型的 framework_resource_* 调用经 ToolService 进入同一个资源服务。'
            '写入时先检查 expected_revision，再由对应资源模块校验并写入 Store。')
W3, G3 = 216, 16
c = [16, 16 + W3 + G3, 16 + 2 * (W3 + G3)]
mid = [x + W3 / 2 for x in c]

cli = e.node(c[0], 16, W3, 52, 'agt 资源命令', '解析参数、文件或 stdin')
web = e.node(c[1], 16, W3, 52, 'Web 资源请求')
model = e.node(c[2], 16, W3, 52, '模型工具调用', 'framework_resource_*')
node = e.node(c[1], 88, W3, 40, 'Web → agt tool call')
gate = e.node(c[2], 88, W3, 40, 'Runtime 工具校验与执行控制')
tools = e.node(c[1], 156, 2 * W3 + G3, 52, 'ToolService', '固定身份、能力与参数检查')
res = e.node(16, 236, 688, 52, 'Resources · agt-service', '统一资源操作')

W4, G4 = 163, 12
c4 = [16 + i * (W4 + G4) for i in range(4)]
y4 = 316
read = e.node(c4[0], y4, W4, 64, 'list / read', '安全视图\nread 返回 revision')
write = e.node(c4[1], y4, W4, 64, 'create / update', '共享写锁内应用修改')
delete = e.node(c4[2], y4, W4, 64, '删除计划与执行', '引用、绑定与确认检查')
cat = e.node(c4[3], y4, W4, 64, '分类管理', '列表、创建、删除空分类')

wx = c4[1] + W4 / 2
CX, CWW = wx - 120, 240
ver = e.node(CX, 412, CWW, 52, 'expected_revision', '若提供，是否匹配？', kind='dec')
conflict = e.node(450, 418, 240, 40, '返回 conflict，不修改', kind='stop')
domain = e.node(CX, 496, CWW, 52, '对应资源写入模块', '类型解析、更新规则与领域校验')
store = e.node(CX, 572, CWW, 52, '对应 Store', '原子写入或暂存提交')
result = e.node(CX, 648, CWW, 40, '返回安全视图与新 revision', kind='end')
e.height = 704

e.edge((mid[0], 70), (mid[0], 234), label='直接调用', at=(mid[0] + 8, 200))
e.edge((mid[1], 70), (mid[1], 86))
e.edge((mid[2], 70), (mid[2], 86))
e.edge((mid[1], 130), (mid[1], 154))
e.edge((mid[2], 130), (mid[2], 154))
e.edge((tools[0] + tools[2] / 2, 210), (tools[0] + tools[2] / 2, 234))
for x in c4:
    e.edge((x + W4 / 2, 290), (x + W4 / 2, y4 - 2))
e.edge((wx, y4 + 65), (wx, 410))
e.edge((CX + CWW + 1, 438), (448, 438), label='否', at=(CX + CWW + 12, 432))
e.edge((wx, 465), (wx, 494), label='是或未提供', at=(wx + 8, 486))
e.edge((wx, 549), (wx, 570))
e.edge((wx, 625), (wx, 646))

# Native refresh of its own capabilities after a tool batch.
r = Sequence('Native 自身能力刷新时序',
             '模型通过 resource_write 修改自身能力后，配置先保存；本批次剩余调用仍用旧能力，批次结束后加载候选能力。'
             '加载成功则切换并报告 applied，失败保留旧能力并报告 failed，取消或失去执行资格则终止。',
             [('M', '模型', None), ('N', 'Native 工具批次', None), ('R', 'Resources', None), ('C', '候选能力加载', None)]
             ).build([
    ('msg', 'M', 'N', 'framework_resource_write\n修改自身能力'),
    ('msg', 'N', 'R', '权限通过后保存配置'),
    ('reply', 'R', 'N', '保存成功与 revision'),
    ('self', 'N', '记录 runtime_application=pending'),
    ('self', 'N', '用旧能力完成本批次剩余调用'),
    ('msg', 'N', 'C', '批次结束，读取 MCP/Skill 候选配置'),
    ('self', 'C', '保留当前模型、Provider\n基础 Prompt 和内置能力'),
    ('self', 'C', '检查模型工具能力，按 Agent 装配加载 MCP'),
    ('block', 'opt', [('MCP 配置或挂载变化', [
        ('self', 'C', '连接服务、获取 schema\n检查工具数量并编译校验器'),
    ])]),
    ('block', 'alt', [
        ('加载成功，且取消与执行资格检查通过', [
            ('reply', 'C', 'N', '候选提示词与可选工具快照'),
            ('self', 'N', '更新执行指纹，切换能力'),
            ('msg', 'N', 'M', '写入结果 + 独立 applied 通知\n+ 当前工具 schema'),
        ]),
        ('加载失败', [
            ('reply', 'C', 'N', '失败原因'),
            ('self', 'N', '保留旧能力，记录失败审计\n不撤销已保存配置'),
            ('msg', 'N', 'M', '写入结果 + 独立 failed 通知\n+ 旧工具 schema'),
        ]),
        ('取消或失去执行资格', [
            ('reply', 'C', 'N', '终止刷新与执行'),
        ]),
    ]),
])

diagrams = {'resource-flow': e, 'resource-refresh': r}
