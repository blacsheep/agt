from svgkit import Diagram

d = Diagram(734, 652, 'agt 整体架构',
            '入口经 agt CLI 与 agt serve 进入服务层；新消息交给执行层的 Scheduler，Runtime 把 Run 交给 Native 或 Harness 执行，'
            '工具调用回到 ToolService；状态保存在资源文件与 SQLite 中，变更事件推送回 Web 服务。')
X = 34  # left gutter for the tool-call return line
c1, c2, c3 = X + 16, X + 186, X + 356  # node columns
W = 140

d.band(X, 12, 512, 92, '入口')
d.node(c1, 40, W, 50, '外部 MCP 客户端', 'Claude、IDE 等')
d.node(c2, 40, W, 50, '浏览器', 'Web 控制台')
d.node(c3, 40, W, 50, 'Node Web 服务', '静态页面、HTTP、SSE')
d.node(c1, 132, W, 44, 'agt serve')
d.node(c3, 132, W, 44, 'agt CLI')

d.band(X, 200, 512, 100, '服务层')
d.node(c1, 230, W, 52, 'ToolService', '工具契约、身份与权限')
d.node(c2, 230, W, 52, 'Resources', '七类配置资源')
d.node(c3, 230, W, 52, 'agt-im', '会话、消息、投递')

d.band(X, 332, 512, 100, '执行层 · agt-runtime')
d.node(c1, 362, W, 52, 'Runtime', 'Run 认领与终态')
d.node(c2, 362, W, 52, 'Scheduler', '准入、排队、并发')
d.node(c3, 362, W, 52, 'WorkItem', '任务、依赖、汇报')

d.band(X, 464, 342, 100, '执行引擎')
d.node(c1, 494, W, 52, 'Native', '内置模型与工具循环')
d.node(c2, 494, W, 52, 'Harness', '外部引擎桥接')

d.node(X, 598, 92, 40, '', '模型 API', kind='ext')
d.node(X + 100, 598, 100, 40, '', '外部 MCP 工具', kind='ext')
d.node(X + 208, 598, 126, 40, '', 'Codex app-server', kind='ext')

d.store(X + 548, 210, 140, 220, '资源文件\nSQLite', 'im · runtime\nmemory')

# Entry and processes
d.edge((c2 + W + 2, 65), (c3 - 2, 65), both=True)
d.edge((X + 86, 92), (X + 86, 130), both=True, label='stdio')
d.edge((X + 426, 92), (X + 426, 130), label='子进程')
d.edge((X + 120, 178), (X + 120, 228))
d.edge((X + 426, 178), (X + 426, 198), label='工具调用、资源命令', at=(X + 434, 192))
# Messages into execution
d.edge((X + 426, 284), (X + 426, 316), (X + 256, 316), (X + 256, 360), label='新消息', at=(X + 270, 311))
d.edge((c2 - 2, 388), (c1 + W + 2, 388))
d.edge((c2 + W + 2, 388), (c3 - 2, 388), both=True)
d.edge((X + 86, 416), (X + 86, 492))
d.edge((X + 130, 416), (X + 130, 448), (X + 256, 448), (X + 256, 492))
# Outside calls
d.edge((X + 56, 548), (X + 48, 596), both=True)
d.edge((X + 118, 548), (X + 146, 596), both=True)
d.edge((X + 268, 548), (X + 268, 596), both=True)
# Storage
d.edge((X + 514, 250), (X + 546, 250))
d.edge((X + 514, 382), (X + 546, 382))
# Returns
d.edge((c1, 520), (X - 12, 520), (X - 12, 256), (c1 - 2, 256), back=True)
d.text(X - 21, 388, '工具调用', 'el bl', 'middle', rotate=-90)
d.edge((X + 618, 208), (X + 618, 65), (c3 + W + 2, 65), back=True,
       label='变更事件\n→ SSE', at=(X + 626, 140))
