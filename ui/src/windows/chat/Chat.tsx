import { useState, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { IconChat, IconSend } from "../../components/icons";
import { STRINGS } from "../../constants/strings";
import type { ChatMessage, SendMessageResponse } from "../../types";

export function Chat() {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const [pinned, setPinned] = useState(false);

  // Auto-scroll to bottom
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, isLoading]);

  const sendMessage = async () => {
    if (!input.trim() || isLoading) return;
    
    const userInput = input.trim();
    const userMsg: ChatMessage = {
      id: crypto.randomUUID(),
      content: userInput,
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
      const response = await invoke<SendMessageResponse>("send_message", { content: userMsg.content });
      const assistantMsg: ChatMessage = {
        id: crypto.randomUUID(),
        content: response.response,
        role: "assistant",
        timestamp: new Date(),
      };
      setMessages(prev => [...prev, assistantMsg]);
    } catch (err) {
      const errorMsg: ChatMessage = {
        id: crypto.randomUUID(),
        content: `Error: ${String(err)}`,
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
      className="flex flex-col h-full overflow-hidden border rounded-lg shadow-xl"
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
      />
      
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
              <div 
                className="px-3 py-2 text-sm whitespace-pre-wrap break-words shadow-sm"
                style={{
                  maxWidth: "75%",
                  borderRadius: msg.role === "user" ? "12px 12px 2px 12px" : "12px 12px 12px 2px",
                  background: msg.role === "user" ? "var(--chat-user-bg)" : "var(--chat-assistant-bg)",
                  color: msg.role === "user" ? "#ffffff" : "var(--text-primary)",
                }}
              >
                {msg.content}
              </div>
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
      <div 
        className="p-3 flex items-end gap-2"
        style={{ borderTop: "1px solid var(--border)" }}
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
          title="Send message"
        >
          <IconSend size={18} />
        </button>
      </div>
    </div>
  );
}
