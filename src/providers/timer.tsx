import { invoke } from '@tauri-apps/api/core';
import { TimerPreview, TimerCreatePreview, TimerExpiredPreview, TimerHintPreview } from '../components/TimerPreviews';
import { registerProvider, dispatchLaunch, type PreviewProps } from './registry';

function TimerPreviewDispatcher({ result, onLaunch, onStopTimer }: PreviewProps) {
  if (result.kind === 'timer-item') {
    return <TimerPreview key={result.id} result={result} onStop={onStopTimer ?? (() => {})} />;
  }
  if (result.kind === 'timer-create') {
    return <TimerCreatePreview result={result} onStart={onLaunch} />;
  }
  if (result.kind === 'timer-expired') {
    return <TimerExpiredPreview label={result.title} onDismiss={onLaunch} />;
  }
  if (result.kind === 'timer-hint') {
    return <TimerHintPreview />;
  }
  return null;
}

registerProvider({
  kinds: ['timer-item', 'timer-create', 'timer-expired', 'timer-hint'],
  Preview: TimerPreviewDispatcher,

  handleLaunch: (result, ctx) => {
    const exec = result.exec;
    if (!exec) {
      if (result.kind === 'timer-create' || result.kind === 'timer-hint') {
        ctx.setQuery('timer ');
        return true;
      }
      return false;
    }
    if (exec.startsWith('timer:create:')) {
      const rest = exec.slice('timer:create:'.length);
      const colon = rest.indexOf(':');
      invoke('create_timer', {
        durationSecs: parseInt(rest.slice(0, colon)),
        label: rest.slice(colon + 1),
      });
      ctx.setQuery('timer');
      ctx.setResults([]);
      return true;
    }
    if (exec.startsWith('timer:stop:')) {
      invoke('stop_timer', { id: parseInt(exec.slice('timer:stop:'.length)) });
      ctx.requery();
      return true;
    }
    if (exec.startsWith('timer:dismiss:')) {
      ctx.removeExpiredTimer(parseInt(exec.slice('timer:dismiss:'.length)));
      return true;
    }
    return false;
  },

  handleKeyDown: (e, result, ctx) => {
    if (e.key === 'Enter' && result?.kind === 'timer-item') {
      e.preventDefault();
      return true;
    }
    if (e.key === 'Delete' && (result?.kind === 'timer-item' || result?.kind === 'timer-expired')) {
      e.preventDefault();
      if (result?.exec) dispatchLaunch(result, ctx);
      return true;
    }
    return false;
  },
});
