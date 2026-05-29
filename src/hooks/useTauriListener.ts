import { useEffect, type DependencyList } from "react";
import { listen } from "@tauri-apps/api/event";

export function useTauriListener<T>(
  event: string,
  handler: (payload: T) => void,
  deps: DependencyList = [],
) {
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    listen<T>(event, e => { if (active) handler(e.payload); })
      .then(fn => { active ? (unlisten = fn) : fn(); });
    return () => { active = false; unlisten?.(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
}
