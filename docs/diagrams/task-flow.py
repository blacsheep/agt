from svgkit import Sequence

d = Sequence('异步任务协作时序',
             'Agent A 创建子任务并登记等待后安全挂起；Agent B 执行子任务，结果持久化后 A 在条件满足时继续，'
             '读取结果并提交汇报，最后以稳定消息键发布到原始会话。',
             [('A', 'Agent A', '请求方'), ('W', 'Work 账本', '与 Scheduler'), ('B', 'Agent B', '执行方'), ('IM', '原始会话', 'IM')]
             ).build([
    ('msg', 'A', 'W', 'task_request 创建子任务'),
    ('reply', 'W', 'A', '返回 task_id'),
    ('msg', 'A', 'W', 'task_await 登记依赖'),
    ('note', 'A', 'W', '保存上下文与已采用配置指纹\n确认安全挂起，释放 A 的执行槽'),
    ('msg', 'W', 'B', '准入后启动子任务 Run'),
    ('reply', 'B', 'W', '成功、失败或其他真实结果'),
    ('self', 'W', '持久化结果、通知与恢复条件'),
    ('msg', 'W', 'A', '条件满足且获准后启动 continuation Run'),
    ('note', 'A', 'W', '核对重新加载的配置指纹与工作目录\n一致才从检查点继续，不重放已完成工具'),
    ('msg', 'A', 'W', '读取结果并提交 task_report'),
    ('msg', 'W', 'IM', '使用稳定消息键发布'),
    ('self', 'W', '确认汇报已送达'),
])
