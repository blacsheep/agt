// Shared by the console and its HTTP boundary. No shell or model command forwarding.
globalThis.AgtCommands = Object.freeze({
  items: Object.freeze([
    { name: 'help', label: $t('命令帮助'), description: $t('查看 Web 已接入的命令与边界。') },
    { name: 'new', label: $t('重置上下文'), description: $t('确认后重置本会话 Agent 的工作上下文，保留聊天记录。') },
    { name: 'clear', label: $t('重置上下文'), description: $t('/new 的别名；不会删除聊天记录或项目文件。') },
    { name: 'model', label: $t('选择模型'), description: $t('读取 Codex 可用模型与档位，仅修改当前执行会话。') },
    { name: 'status', label: $t('会话运行状态'), description: $t('查看本会话 Agent 的启用状态、执行方式和最近运行。') },
    { name: 'compact', requires: 'codex', label: $t('压缩上下文'), description: $t('调用 Codex 压缩并等待完成，保留聊天记录。') },
    { name: 'rename', requires: 'codex', label: $t('会话命名'), description: $t('设置 Codex 会话名称，保留 Session ID。') },
    { name: 'skills', requires: 'codex', label: $t('查看 Skills'), description: $t('读取此 Harness 项目的 Codex Skills。') },
    { name: 'mcp', requires: 'codex', label: $t('查看 MCP'), description: $t('读取 Codex MCP 服务器和认证状态。') },
    { name: 'permissions', requires: 'codex', label: $t('查看权限'), description: $t('查看会话保存的文件权限，不提升权限。') },
    { name: 'timers', label: $t('定时任务'), description: $t('查看本会话的定时任务和下次触发时间，可以取消。') },
    { name: 'stop', label: $t('停止本会话 Agent'), description: $t('确认后结束本轮讨论，停止相关执行。') },
  ]),
  parse(text) {
    if (typeof text !== 'string') return null;
    const value = text.trim();
    if (value === '/') return {type:'command',name:'help',args:''};
    if (value.startsWith('//')) return { type: 'literal', text: value.slice(1) };
    const match = /^\/([a-z][a-z0-9_-]*)(?:[ \t]+([^\r\n]*))?$/i.exec(value);
    return match ? { type: 'command', name: match[1].toLowerCase(), args: (match[2] || '').trim() } : null;
  },
});
