/* Persistent workspace modes; no approval enumeration or bulk decision requests. */
window.AgtPermissionMode = (() => {
  async function mount(host, canEdit) {
    host.textContent=$t('正在读取权限处理模式…');
    try {
      let {config}=await api('config');
      if(!host.isConnected)return;
      const p=config.permissions;
      const modes=(value,review)=>`${review?`<option value="model_review" ${value==='model_review'?'selected':''}>${$t('模型审核，异常转人工')}</option>`:''}<option value="manual" ${value==='manual'?'selected':''}>${$t('人工审批')}</option><option value="auto_allow" ${value==='auto_allow'?'selected':''}>${$t('自动放行后续请求')}</option>`;
      const models=items('model').map(ref), current=p.reviewer_model||'';
      if(current&&!models.includes(current))models.push(current);
      host.innerHTML=`<form class="permission-mode-form"><h3>${$t('权限处理模式')}</h3><p class="form-note">${$t('工作空间范围，保存后持续生效，并应用于当前等待及后续请求；关闭页面仍有效。已作出的决定不会撤回，Codex 沙箱范围不变。')}</p><label>${$t('Native 工具')}<select name="native" ${canEdit?'':'disabled'}>${modes(p.mode,true)}</select></label><label>Codex / Harness<select name="harness" ${canEdit?'':'disabled'}>${modes(p.harness_mode||'manual',false)}</select></label><label>${$t('Native 审核模型')}<select name="reviewer" ${canEdit?'':'disabled'}><option value="">${$t('未配置，转人工')}</option>${models.map(m=>`<option value="${esc(m)}" ${m===current?'selected':''}>${esc(m)}</option>`).join('')}</select></label>${canEdit?`<button type="submit" class="primary">${$t('保存处理模式')}</button>`:`<p>${$t('观察身份只读，请切换为控制台操作者修改模式。')}</p>`}<p class="permission-mode-feedback" role="status"></p></form>`;
      const form=host.querySelector('form'),feedback=host.querySelector('.permission-mode-feedback');
      form.onsubmit=async event=>{
        event.preventDefault();if(!canEdit||state.viewer!==state.boot?.principal)return;
        const button=form.querySelector('button');button.disabled=true;
        try {
          const next=structuredClone(config), data=new FormData(form);
          next.permissions.mode=data.get('native');next.permissions.harness_mode=data.get('harness');next.permissions.reviewer_model=data.get('reviewer')||null;
          config=await api('config',{config:next,expected:config});
          if(host.isConnected)feedback.textContent=$t('已保存，当前等待和后续请求将按此模式处理。')+(next.permissions.mode==='model_review'&&!next.permissions.reviewer_model?$t('尚未配置审核模型，Native 将转人工。'):'');
        }catch(error){if(host.isConnected)feedback.textContent=errorText(error)+$t(' 请重新打开面板读取最新配置。');}
        finally{button.disabled=false;}
      };
    }catch(error){if(host.isConnected)host.textContent=$t('权限模式读取失败：{0}', [errorText(error)]);}
  }
  return {mount};
})();
