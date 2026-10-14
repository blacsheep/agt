"""Diagrams in the user guide, drawn once for each language with the same layout."""
from svgkit import Diagram, Sequence

TEXT = {
    'en': {
        'fit.title': 'How agt fits together',
        'fit.desc': 'You chat in conversations; the scheduler runs agents for new messages. Agents reply or delegate in '
                    'conversations, create and change resources that configure them, and call MCP servers, Codex and the shell.',
        'you': 'You', 'conv': 'Conversations', 'sched': 'Scheduler', 'agents': 'Agents', 'res': 'Resources',
        'tools': 'MCP servers, Codex, shell', 'chat': 'chat', 'messages': 'messages', 'runs': 'runs',
        'reply': 'reply / delegate', 'change': 'create & change', 'configure': 'configure', 'call': 'call',
        'bind.title': 'Definition, binding and identity',
        'bind.desc': 'A binding connects an agent definition to an identity, and the identity is a member of conversations.',
        'def': 'Agent definition', 'id': 'Identity', 'convs': 'Conversations', 'binding': 'binding', 'member': 'member of',
        'task.title': 'How a task flows',
        'task.desc': 'The lead requests a task; the scheduler queues it and runs the worker, posts the result in the task '
                     'conversation and notifies the lead, which reviews it and files a report in the original conversation.',
        'lead': 'Lead', 'worker': 'Worker',
        't1': '1 task_request(assignee, conversation, input, request_key)', 't1r': 'queued',
        't2': '2 run the task', 't2r': 'result', 't2s': 'post the result in the task conversation',
        't3': '3 notify: result ready', 't3s': 'review the result (task_get)',
        't4': '4 task_report(body) to the original conversation',
    },
    'zh': {
        'fit.title': 'agt 各部分如何配合',
        'fit.desc': '你在会话中对话，调度器为新消息执行 Agent。Agent 在会话中回复或委派，创建和修改配置自己的资源，'
                    '并调用 MCP 服务、Codex 和 Shell。',
        'you': '你', 'conv': '会话', 'sched': '调度器', 'agents': 'Agent', 'res': '资源',
        'tools': 'MCP 服务、Codex、Shell', 'chat': '对话', 'messages': '消息', 'runs': '执行',
        'reply': '回复 / 委派', 'change': '创建与修改', 'configure': '配置', 'call': '调用',
        'bind.title': 'Agent 定义、绑定与身份',
        'bind.desc': '绑定把 Agent 定义和身份连接起来，身份是会话的成员。',
        'def': 'Agent 定义', 'id': '身份', 'convs': '会话', 'binding': '绑定', 'member': '成员',
        'task.title': '任务的流程',
        'task.desc': 'lead 发起任务；调度器排队并执行执行者，把结果发到任务所在会话并通知 lead，lead 审阅后向原会话提交汇报。',
        'lead': 'Lead', 'worker': '执行者',
        't1': '1 task_request(assignee, conversation, input, request_key)', 't1r': 'queued',
        't2': '2 执行任务', 't2r': '结果', 't2s': '把结果发到任务所在会话',
        't3': '3 通知：结果已就绪', 't3s': '审阅结果（task_get）',
        't4': '4 task_report(body) 发往原会话',
    },
}


def fits(t):
    d = Diagram(720, 300, t['fit.title'], t['fit.desc'])
    you = d.node(16, 24, 100, 40, t['you'], kind='end')
    conv = d.node(176, 24, 160, 40, t['conv'])
    sched = d.node(416, 24, 160, 40, t['sched'])
    agents = d.node(416, 128, 160, 40, t['agents'])
    tools = d.node(616, 120, 96, 56, '', t['tools'], kind='ext')
    d.store(416, 218, 160, 66, t['res'])
    d.edge((117, 44), (174, 44), label=t['chat'], at=(146, 38), anchor='middle')
    d.edge((337, 44), (414, 44), label=t['messages'], at=(376, 38), anchor='middle')
    d.edge((496, 65), (496, 126), label=t['runs'], at=(504, 100))
    d.edge((415, 148), (256, 148), (256, 66), label=t['reply'], at=(264, 140))
    d.edge((577, 148), (614, 148), label=t['call'], at=(596, 142), anchor='middle')
    d.edge((470, 169), (470, 216), label=t['change'], at=(462, 198), anchor='end')
    d.edge((522, 216), (522, 171), label=t['configure'], at=(530, 198))
    return d


def binds(t):
    d = Diagram(720, 92, t['bind.title'], t['bind.desc'])
    a = d.node(16, 20, 210, 52, t['def'], 'agents/default/reviewer')
    b = d.node(296, 20, 150, 52, t['id'], 'reviewer')
    c = d.node(516, 20, 188, 52, t['convs'])
    d.edge((227, 46), (294, 46), label=t['binding'], at=(260, 40), anchor='middle')
    d.edge((447, 46), (514, 46), label=t['member'], at=(480, 40), anchor='middle')
    return d


def tasks(t):
    return Sequence(t['task.title'], t['task.desc'],
                    [('L', t['lead'], None), ('S', t['sched'], None), ('W', t['worker'], None)]).build([
        ('msg', 'L', 'S', t['t1']),
        ('reply', 'S', 'L', t['t1r']),
        ('msg', 'S', 'W', t['t2']),
        ('reply', 'W', 'S', t['t2r']),
        ('self', 'S', t['t2s']),
        ('msg', 'S', 'L', t['t3']),
        ('self', 'L', t['t3s']),
        ('msg', 'L', 'S', t['t4']),
    ])


diagrams = {}
for lang, t in TEXT.items():
    diagrams[f'guide/how-it-fits.{lang}'] = fits(t)
    diagrams[f'guide/agent-binding.{lang}'] = binds(t)
    diagrams[f'guide/task-flow.{lang}'] = tasks(t)
