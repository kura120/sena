import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { STRINGS } from "../../constants/strings";
import { IconChevronRight, IconChevronDown } from "../../components/icons";
import "./ModelPanel.css";
import type { 
  DebugSnapshot, 
  LocalModel, 
  OllamaModel
} from "../../types";

const POLLING_INTERVAL = 2000;

const AGENT_NAMES = [
  "reactive-loop",
  "reasoning",
  "memory",
  "file",
  "screen",
  "process",
  "browser",
  "peripheral",
  "tacet"
];

function formatFlow(tokensPerSec: number): string {
    return tokensPerSec ? `${tokensPerSec.toFixed(1)} tok/s` : "0.0 tok/s";
}

function getTier(totalMb: number): string {
    if (totalMb < 4096) return "Low";
    if (totalMb < 8192) return "Mid";
    if (totalMb < 16384) return "High";
    return "Ultra";
}

export function ModelPanel() {
    const [snapshot, setSnapshot] = useState<DebugSnapshot | null>(null);
    const [assignments, setAssignments] = useState<Record<string, string>>({});
    const [localModels, setLocalModels] = useState<LocalModel[]>([]);
    const [ollamaModels, setOllamaModels] = useState<OllamaModel[]>([]);
    const [libraryOpen, setLibraryOpen] = useState(false);
    const [activeTab, setActiveTab] = useState<"local" | "ollama">("local");
    const [extracting, setExtracting] = useState<string | null>(null); // digest being extracted

    // Load initial data
    useEffect(() => {
        refreshData();
        
        const interval = setInterval(() => {
            invoke<DebugSnapshot>("get_debug_snapshot")
                .then(setSnapshot)
                .catch(err => console.error("Failed to poll snapshot:", err));
        }, POLLING_INTERVAL);

        return () => clearInterval(interval);
    }, []);

    // Listen for progress
    useEffect(() => {
        const unlisten = listen<{ model_name: string, progress: number, copied_bytes: number, total_bytes: number }>("model-extract-progress", (_event) => {
            // Extraction is tracked via the extracting state; progress events available for future enhancement
        });

        return () => { unlisten.then(fn => fn()); };
    }, []);

    const refreshData = async () => {
        try {
            const [local, ollama, assigns, snap] = await Promise.all([
                invoke<LocalModel[]>("list_local_models"),
                invoke<OllamaModel[]>("list_ollama_models"),
                invoke<Record<string, string>>("get_agent_model_assignments"),
                invoke<DebugSnapshot>("get_debug_snapshot")
            ]);
            setLocalModels(local);
            setOllamaModels(ollama);
            setAssignments(assigns);
            setSnapshot(snap);
        } catch (err) {
            console.error("Failed to load initial data:", err);
        }
    };

    const handleAssign = async (agent: string, modelId: string) => {
        try {
            await invoke("set_agent_model_assignment", { agent, modelId });
            setAssignments(prev => ({ ...prev, [agent]: modelId }));
        } catch (err) {
            console.error(`Failed to assign ${modelId} to ${agent}:`, err);
        }
    };

    const handleSwitch = async (modelPath: string) => {
        try {
            await invoke("switch_model", { modelPath });
            refreshData(); // Refresh to update is_active status
        } catch (err) {
            console.error("Failed to switch model:", err);
        }
    };

    const handleDelete = async (path: string) => {
        if (!confirm(STRINGS.MODEL_CONFIRM_DELETE)) return;
        try {
            await invoke("delete_local_model", { path });
            refreshData();
        } catch (err) {
            console.error("Failed to delete model:", err);
        }
    };

    const handleExtract = async (model: OllamaModel) => {
        setExtracting(model.blob_digest);
        try {
            const extractName = `${model.name}-${model.tag}`;
            await invoke("extract_ollama_model", { blobDigest: model.blob_digest, modelName: extractName });
            setExtracting(null);
            refreshData();
        } catch (err) {
            console.error("Failed to extract model:", err);
            setExtracting(null);
        }
    };

    const inf = snapshot?.inference_stats;
    const vram = snapshot?.vram;
    const vramPercent = vram ? (vram.used_mb / vram.total_mb) * 100 : 0;
    const vramClass = vramPercent > 95 ? "critical" : vramPercent > 80 ? "warning" : "good";
    
    // Find the active model's metadata from local models
    const activeModelMeta = localModels.find(m => m.is_active);
    
    // Sort models
    const sortedLocal = [...localModels].sort((a, b) => {
        if (a.is_active === b.is_active) return a.display_name.localeCompare(b.display_name);
        return a.is_active ? -1 : 1;
    });

    return (
        <div className="model-panel">
            <h2 className="panel-title">{STRINGS.PANEL_MODEL}</h2>
            
            {/* Active Model Section */}
            <div className="section active-model">
                <div className="section-header">{STRINGS.MODEL_ACTIVE}</div>
                <div className="card active-card">
                    {inf && inf.active_model ? (
                        <>
                            <div className="row title-row">
                                <span className="model-name" title={inf.active_model}>{inf.model_display_name}</span>
                                {activeModelMeta && (
                                    <span className="model-arch-quant">
                                        {activeModelMeta.architecture} • {activeModelMeta.quantization}
                                    </span>
                                )}
                            </div>
                            <div className="row stats-row">
                                <span className="stat">{formatFlow(inf.tokens_per_second)}</span>
                                <span className="stat">{inf.total_completions} {STRINGS.MODEL_COMPLETIONS}</span>
                            </div>
                            <div className="vram-container">
                                <div className="vram-bar">
                                    <div 
                                        className={`vram-fill ${vramClass}`} 
                                        style={{ width: `${vramPercent}%` }}
                                    />
                                </div>
                                <div className="vram-text">
                                    {STRINGS.MODEL_VRAM} {vram ? (vram.used_mb/1024).toFixed(1) : 0}/
                                    {vram ? (vram.total_mb/1024).toFixed(0) : 0}GB
                                    <span className="tier-badge">{vram ? getTier(vram.total_mb) : "-"}</span>
                                </div>
                            </div>
                        </>
                    ) : (
                        <div className="empty-state">{STRINGS.MODEL_NO_ACTIVE}</div>
                    )}
                </div>
            </div>

            {/* Agent Assignments */}
            <div className="section assignments">
                <div className="section-header">{STRINGS.MODEL_AGENTS}</div>
                <div className="agents-table">
                    {AGENT_NAMES.map(agent => (
                        <div key={agent} className="agent-row">
                            <span className="agent-name">{agent}</span>
                            <select 
                                className="model-select"
                                value={assignments[agent] || "auto"}
                                onChange={(e) => handleAssign(agent, e.target.value)}
                                aria-label={`Model assignment for ${agent}`}
                            >
                                <option value="auto">{STRINGS.MODEL_AUTO}</option>
                                {sortedLocal.map(m => (
                                    <option key={m.filename} value={m.filename}>
                                        {m.display_name}
                                    </option>
                                ))}
                            </select>
                        </div>
                    ))}
                </div>
            </div>

            {/* Model Library */}
            <div className="section library">
                <button 
                    type="button"
                    className="library-toggle"
                    onClick={() => setLibraryOpen(!libraryOpen)}
                    aria-expanded={libraryOpen}
                    aria-controls="model-library-content"
                >
                    {libraryOpen ? <IconChevronDown size={14} /> : <IconChevronRight size={14} />}
                    {STRINGS.MODEL_LIBRARY}
                </button>
                
                {libraryOpen && (
                    <div className="library-content" id="model-library-content">
                        <div className="tabs" role="tablist">
                            <button 
                                type="button"
                                role="tab"
                                className={`tab ${activeTab === 'local' ? 'active' : ''}`}
                                onClick={() => setActiveTab('local')}
                                aria-selected={activeTab === 'local'}
                            >
                                {STRINGS.MODEL_TAB_LOCAL} ({localModels.length})
                            </button>
                            <button 
                                type="button"
                                role="tab"
                                className={`tab ${activeTab === 'ollama' ? 'active' : ''}`}
                                onClick={() => setActiveTab('ollama')}
                                aria-selected={activeTab === 'ollama'}
                            >
                                {STRINGS.MODEL_TAB_OLLAMA} ({ollamaModels.length})
                            </button>
                        </div>

                        <div className="model-list">
                            {activeTab === 'local' ? (
                                sortedLocal.map(m => (
                                    <div key={m.path} className={`model-item ${m.is_active ? 'active' : ''}`}>
                                        <div className="model-info">
                                            <div className="model-main">
                                                <span className="name" title={m.filename}>{m.display_name}</span>
                                                {m.is_active && <span className="active-badge">{STRINGS.MODEL_BADGE_ACTIVE}</span>}
                                            </div>
                                            <div className="model-meta">
                                                {m.architecture} • {m.quantization} • {m.size_gb.toFixed(1)}GB
                                            </div>
                                        </div>
                                        <div className="model-actions">
                                            <button 
                                                type="button"
                                                className="action-btn"
                                                disabled={m.is_active}
                                                onClick={() => handleSwitch(m.path)}
                                            >
                                                {STRINGS.MODEL_SWITCH}
                                            </button>
                                            <button 
                                                type="button"
                                                className="action-btn danger"
                                                disabled={m.is_active}
                                                onClick={() => handleDelete(m.path)}
                                            >
                                                {STRINGS.MODEL_DELETE}
                                            </button>
                                        </div>
                                    </div>
                                ))
                            ) : (
                                ollamaModels.map(m => (
                                    <div key={m.blob_digest} className="model-item">
                                        <div className="model-info">
                                            <div className="model-main">
                                                <span className="name" title={m.name}>{m.name}</span>
                                                <span className="tag">{m.tag}</span>
                                            </div>
                                            <div className="model-meta">
                                                {m.architecture} • {m.size_gb.toFixed(1)}GB
                                                {m.lora_compatible && <span className="feature-badge lora">{STRINGS.MODEL_BADGE_LORA}</span>}
                                                {m.chain_of_thought_support && <span className="feature-badge cot">{STRINGS.MODEL_BADGE_COT}</span>}
                                            </div>
                                        </div>
                                        <div className="model-actions">
                                            {m.is_extracted ? (
                                                <span className="extracted-label">{STRINGS.MODEL_BADGE_EXTRACTED}</span>
                                            ) : (
                                                <button 
                                                    type="button"
                                                    className="action-btn primary"
                                                    disabled={extracting === m.blob_digest}
                                                    onClick={() => handleExtract(m)}
                                                >
                                                    {extracting === m.blob_digest ? STRINGS.MODEL_EXTRACTING : STRINGS.MODEL_EXTRACT}
                                                </button>
                                            )}
                                        </div>
                                    </div>
                                ))
                            )}
                        </div>
                    </div>
                )}
            </div>
        </div>
    );
}
