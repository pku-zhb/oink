import { useEffect, useRef, useState } from 'react';
import { Button, Dialog } from 'oink-ui';
import type { DialogProps } from 'oink-ui';
import { agent } from '../api/store';
import { inTauri } from '../api/transport';
import { physicalDesktop } from '../api/physicalDesktop';
import type { Confirmation } from '../api/physicalDesktop';
import { tr } from '../i18n';

/** Visible participants can confirm without navigating to the settings page. */
export function PhysicalConfirmation({container}:{container:HTMLElement}) {
  const [info,setInfo]=useState<Confirmation|null>(null);
  const [now,setNow]=useState(Date.now());
  const [sending,setSending]=useState(false);
  const [error,setError]=useState('');
  const [dismissed,setDismissed]=useState<string|null>(null);
  const current=useRef<Confirmation|null>(null);
  const check=useRef<()=>Promise<void>>(async()=>{});
  useEffect(()=>{
    let alive=true,pending=false;
    const read=async()=>{
      if(pending)return;pending=true;
      try {
        const next=await physicalDesktop<Confirmation|null>({action:'confirmation'});
        if(alive){current.current=next;setInfo(next);}
      } catch { /* Older backends have no global physical confirmation. */ }
      finally {pending=false;}
    };
    check.current=read;void read();
    const timer=window.setInterval(()=>{setNow(Date.now());if(current.current?.phase==='pending')void read();},500);
    const unsubscribe=agent.onEvent?.(event=>{if(event.type==='show_window')void read();});
    return()=>{alive=false;window.clearInterval(timer);unsubscribe?.();};
  },[]);
  useEffect(()=>{setSending(false);setError('');setDismissed(null);},[info?.operation,info?.phase]);
  useEffect(()=>{
    if(!info||!inTauri)return;
    const bounds=info.phase==='pending'||info.phase==='kept'?info.bounds:info.previous_bounds;
    if(!bounds)return;
    let cancelled=false,restore:(()=>Promise<void>)|undefined;
    const task=(async()=>{
      const [{getCurrentWindow},{LogicalPosition,PhysicalPosition}]=await Promise.all([import('@tauri-apps/api/window'),import('@tauri-apps/api/dpi')]);
      const win=getCurrentWindow();
      if(await win.isMaximized())await win.unmaximize();
      const [size,scale,wasTop]=await Promise.all([win.outerSize(),win.scaleFactor(),win.isAlwaysOnTop()]);
      if(cancelled)return;
      const unit=info.macos?scale:1;
      const x=bounds.x+Math.max(0,(bounds.w-size.width/unit)/2);
      const y=bounds.y+Math.max(0,(bounds.h-size.height/unit)/2);
      await win.setPosition(info.macos?new LogicalPosition(x,y):new PhysicalPosition(Math.round(x),Math.round(y)));
      if(info.phase==='pending'||info.phase==='error'){
        restore=()=>win.setAlwaysOnTop(wasTop);
        await win.setAlwaysOnTop(true);await win.unminimize();await win.show();
        // Windows may decline foreground focus. Do not borrow UIAccess grants.
        await win.setFocus().catch(()=>{});
      }
    })().catch(()=>{});
    return()=>{cancelled=true;void task.then(()=>restore?.());};
  },[info?.operation,info?.phase]);
  const seconds=Math.max(0,Math.ceil(((info?.deadline_ms??now)-now)/1000));
  const pending=info?.phase==='pending';
  const failed=info?.phase==='error'&&dismissed!==info.operation;
  const decide=async(action:'keep'|'revert'|'restore')=>{
    if(!info||sending)return;setSending(true);setError('');
    try {
      await physicalDesktop(action==='restore'?{action}:{action,operation:info.operation});
      await check.current();
      if(action==='restore')setSending(false);
    } catch(e){setError(e instanceof Error?e.message:String(e));setSending(false);}
  };
  const contained={contained:true} as Partial<DialogProps>;
  return <Dialog {...contained} open={!!pending||!!failed} container={container} dismissible={false} size={420}
    title={pending?tr('保留这个显示布局？'):tr('部分配置需要恢复')}
    onOpenChange={open=>{if(!open&&pending)void decide('revert');}}
    onKeyDown={event=>{if(event.key==='Escape'&&pending){event.stopPropagation();void decide('revert');}}}
    actions={pending?<><Button disabled={sending||seconds===0} onClick={()=>{void decide('revert');}}>{tr('恢复原布局')}</Button><Button variant="accent" disabled={sending||seconds===0} onClick={()=>{void decide('keep');}}>{sending?tr('正在确认布局'):tr('保留布局')}</Button></>:<><Button onClick={()=>setDismissed(info?.operation??null)}>{tr('关闭')}</Button><Button variant="accent" disabled={sending} onClick={()=>{void decide('restore');}}>{tr('恢复切换前配置')}</Button></>}>
    <p>{pending?tr('{0} 秒后恢复原布局',seconds):tr('请先恢复切换前配置，再尝试新的布局。')}</p>
    {error&&<p role="alert">{error}</p>}
  </Dialog>;
}
