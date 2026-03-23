import { useState, useRef, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { IconChat, IconSend } from "../../components/icons";
import { STRINGS } from "../../constants/strings";
import type { ParsedChatMessage, SendMessageResponse, DebugSnapshot } from "../../types";
import { ThoughtBlock } from "./ThoughtBlock";
import { useWindowDragSave } from "../../hooks/useWindowDragSave";
import { useOverlayAnimation } from "../../hooks/useOverlayAnimation";
import "./Chat.css";

export function Chat() {
  const [messages, setMessages] = useState<ParsedChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const [pinned, setPinned] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [reactiveLoopStatus, setReactiveLoopStatus] = useState<string>("Unknown");

  useWindowDragSave();
  const panelClass = useOverlayAnimation();

  // Poll reactive-loop status
  const checkStatus = useCallback(() => {
    invoke<DebugSnapshot>("get_debug_snapshot").then((snapshot) => {
      const rl = snapshot.subsystems.find(s => s.name === "reactive-loop");
      if (rl) setReactiveLoopStatus(rl.status);
    }).catch(() => { /* backend not ready yet */ });
  }, []);

  useEffect(() => {
    checkStatus();
    const interval = setInterval(checkStatus, 3000);
    return () => clearInterval(interval);
  }, [checkStatus]);

  // Auto-scroll to bottom
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, isLoading]);

  const sendMessage = async () => {
    if (!input.trim() || isLoading) return;
    
    const userInput = input.trim();
    const userMsg: ParsedChatMessage = {
      id: crypto.randomUUID(),
      pre_thought_text: null,
      thought_content: null,
      final_response: userInput,
      chain_of_thought_supported: false,
      role: "user",
      timestamp: new Date(),
    };
    
    setMessages(prev => [...prev, userMsg]);
    setInput("");
    setIsLoading(true);
    
    // Reset textarea height
    const textarea = document.querySelector('textarea');
    if (textarea) textarea.style.height = 'auto';
    
    try {
      const response = await invoke<SendMessageResponse>("send_message", { content: userMsg.final_response });
      const assistantMsg: ParsedChatMessage = {
        id: crypto.randomUUID(),
        pre_thought_text: response.pre_thought_text,
        thought_content: response.thought_content,
        final_response: response.response,
        chain_of_thought_supported: response.chain_of_thought_supported,
        role: "assistant",
        timestamp: new Date(),
        model_id: response.model_id,
        latency_ms: response.latency_ms,
      };
      setMessages(prev => [...prev, assistantMsg]);
    } catch (err) {
      const errorMsg: ParsedChatMessage = {
        id: crypto.randomUUID(),
        pre_thought_text: null,
        thought_content: null,
        final_response: `${STRINGS.CHAT_ERROR_PREFIX}: ${String(err)}`,
        chain_of_thought_supported: false,
        role: "assistant",
        timestamp: new Date(),
      };
      setMessages(prev => [...prev, errorMsg]);
    } finally {
      setIsLoading(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  };

  return (
    <div 
      className={`flex flex-col ${collapsed ? '' : 'h-full'} overflow-hidden border rounded-lg shadow-xl panel-glass ${panelClass}`}
      style={{
        background: "var(--bg-panel)",
        borderColor: "var(--border)",
        borderRadius: "var(--radius)",
      }}
    >
      <TitleBar 
        icon={<IconChat size={14} />} 
        title={STRINGS.PANEL_CHAT}
        pinned={pinned}
        onPinToggle={() => setPinned(!pinned)}
        collapsed={collapsed}
        onCollapseToggle={() => setCollapsed(c => !c)}
      />
      {!collapsed && (
      <>
      {/* Messages Area */}
      <div className="flex-1 overflow-y-auto p-3 flex flex-col gap-2 scrollbar-thin">
        {messages.length === 0 ? (
          <div className="flex items-center justify-center h-full text-sm" style={{ color: "var(--text-muted)" }}>
            {STRINGS.CHAT_EMPTY_STATE}
          </div>
        ) : (
          messages.map((msg) => (
            <div 
              key={msg.id} 
              className={`flex flex-col ${msg.role === "user" ? "items-end" : "items-start"}`}
            >
              {msg.role === "user" ? (
                <div 
                  className="px-3 py-2 text-sm whitespace-pre-wrap break-words shadow-sm"
                  style={{
                    maxWidth: "75%",
                    borderRadius: "12px 12px 2px 12px",
                    background: "var(--chat-user-bg)",
                    color: "var(--chat-user-text)",
                  }}
                >
                  {msg.final_response}
                </div>
              ) : (
                <div className="flex flex-col items-start gap-1 max-w-[90%]">
                  {/* Pre-thought text */}
                  {msg.pre_thought_text && (
                    <div 
                      className="px-3 py-2 text-sm whitespace-pre-wrap break-words shadow-sm"
                      style={{
                        borderRadius: "12px 12px 12px 2px",
                        background: "var(--chat-assistant-bg)",
                        color: "var(--text-primary)",
                      }}
                    >
                      {msg.pre_thought_text}
                    </div>
                  )}
                  {/* Chain-of-thought block */}
                  {msg.thought_content && (
                    <ThoughtBlock content={msg.thought_content} />
                  )}
                  {/* Final response */}
                  {msg.final_response && (
                    <div 
                      className="px-3 py-2 text-sm whitespace-pre-wrap break-words shadow-sm"
                      style={{
                        borderRadius: "12px 12px 12px 2px",
                        background: "var(--chat-assistant-bg)",
                        color: "var(--text-primary)",
                      }}
                    >
                      {msg.final_response}
                    </div>
                  )}
                  {/* Not supported label */}
                  {!msg.chain_of_thought_supported && !msg.thought_content && (
                    <span className="cot-not-supported">{STRINGS.COT_NOT_SUPPORTED}</span>
                  )}
                </div>
              )}
              <span className="text-[10px] mt-1 px-1 opacity-70" style={{ color: "var(--text-muted)" }}>
                {msg.timestamp.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
              </span>
            </div>
          ))
        )}

        {/* Typing Indicator */}
        {isLoading && (
          <div className="flex flex-col items-start animate-fade-in">
            <div 
              className="px-3 py-3 flex items-center gap-1"
              style={{
                borderRadius: "12px 12px 12px 2px",
                background: "var(--chat-assistant-bg)",
                width: "fit-content"
              }}
            >
              <div className="typing-dot" style={{ animationDelay: "0ms" }} />
              <div className="typing-dot" style={{ animationDelay: "200ms" }} />
              <div className="typing-dot" style={{ animationDelay: "400ms" }} />
            </div>
            <span className="text-[10px] mt-1 px-1" style={{ color: "var(--text-muted)" }}>
              {STRINGS.CHAT_TYPING}
            </span>
          </div>
        )}
        <div ref={messagesEndRef} />
      </div>

      {/* Input Area */}
      {reactiveLoopStatus !== "Ready" && (
        <div
          className="px-3 py-1 text-[11px]"
          style={{ color: "var(--status-unavailable)", borderTop: "1px solid var(--border)" }}
        >
          {STRINGS.CHAT_REACTIVE_LOOP_PREFIX}: {reactiveLoopStatus.toLowerCase()}
        </div>
      )}
      <div 
        className="p-3 flex items-end gap-2"
        style={{ borderTop: reactiveLoopStatus === "Ready" ? "1px solid var(--border)" : "none" }}
      >
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={STRINGS.CHAT_INPUT_PLACEHOLDER}
          className="flex-1 bg-transparent resize-none outline-none text-sm max-h-24 overflow-y-auto py-1"
          style={{ color: "var(--text-primary)" }}
          rows={1}
          onInput={(e) => {
            const target = e.target as HTMLTextAreaElement;
            target.style.height = "auto";
            target.style.height = Math.min(target.scrollHeight, 96) + "px"; // max 4 lines approx
          }}
        />
        <button
          onClick={sendMessage}
          disabled={!input.trim() || isLoading}
          className="p-2 rounded-lg transition-all active:scale-95"
          style={{ 
            color: (!input.trim() || isLoading) ? "var(--text-muted)" : "var(--text-primary)",
            opacity: (!input.trim() || isLoading) ? 0.5 : 1,
            cursor: (!input.trim() || isLoading) ? "not-allowed" : "pointer"
          }}
          onMouseEnter={(e) => {
            if (input.trim() && !isLoading) e.currentTarget.style.background = "var(--bg-hover)";
          }}
          onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}
          title={STRINGS.CHAT_SEND}
        >
          <IconSend size={18} />
        </button>
      </div>
      </>
      )}
    </div>
  );
}
