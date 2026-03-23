import { useEffect, useRef, useCallback } from 'react';

export interface WsEvent {
  id: string;
  type: string;
  payload: any;
  timestamp: string;
}

/**
 * 建立到 /ws 的 WebSocket 长连接，接收服务端实时事件推送。
 * 断线后 3 秒自动重连，组件卸载时自动清理连接与定时器。
 *
 * 使用方式：在需要实时更新的页面中调用，传入事件处理回调即可。
 * 路由注册在后续合并步骤中完成，当前仅提供 hook 实现。
 */
export function useWebSocket(onEvent: (event: WsEvent) => void) {
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimer = useRef<number | null>(null);

  const connect = useCallback(() => {
    // 根据页面协议自动选择 ws/wss，适配 TLS 终结后的 HTTPS 环境
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const ws = new WebSocket(`${protocol}//${window.location.host}/ws`);

    ws.onmessage = (e) => {
      try {
        const event: WsEvent = JSON.parse(e.data);
        onEvent(event);
      } catch (err) {
        console.warn('Failed to parse WebSocket message:', err);
      }
    };

    ws.onclose = () => {
      reconnectTimer.current = window.setTimeout(connect, 3000);
    };

    ws.onerror = () => {
      ws.close();
    };

    wsRef.current = ws;
  }, [onEvent]);

  useEffect(() => {
    connect();
    return () => {
      if (wsRef.current) wsRef.current.close();
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current);
    };
  }, [connect]);
}
