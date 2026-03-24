import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from '@tauri-apps/api/window';
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { useOverlayAnimation } from "../../hooks/useOverlayAnimation";
import { useWindowDragSave } from "../../hooks/useWindowDragSave";
import { STRINGS } from "../../constants/strings";
import { IconChevronRight, IconChevronDown, IconFolder, IconModel } from "../../components/icons";
import { Dropdown } from "../../components/Dropdown/Dropdown";
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

function truncatePath(path: string, maxLen: number): string {
    if (path.length <= maxLen) return path;
    return "..." + path.slice(path.length - maxLen + 3);
}

export function ModelPanel() {
    const [snapshot, setSnapshot] = useState<DebugSnapshot | null>(null);
    const [assignments, setAssignments] = useState<Record<string, string>>({});
    const [localModels, setLocalModels] = useState<LocalModel[]>([]);
    const [ollamaModels, setOllamaModels] = useState<OllamaModel[]>([]);
     const [libraryOpen, setLibraryOpen] = useState(false);
    const [activeTab, setActiveTab] = useState<"local" | "ollama">("local");
    const [extracting, setExtracting] = useState<string | null>(null);
    const [extractError, setExtractError] = useState<string | null>(null);
    const [localSearch, setLocalSearch] = useState("");
    const [ollamaSearch, setOllamaSearch] = useState("");
    const [ollamaDir, setOllamaDir] = useState<string>("");
    const [pinned, setPinned] = useState(false);
    const [collapsed, setCollapsed] = useState(false);

    useWindowDragSave();
    const panelClass = useOverlayAnimation();

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

    // Auto-refresh local models when window regains focus (e.g., after opening models folder)
    useEffect(() => {
        const unlisten = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
            if (focused) {
                refreshLocalModels();
            }
        });
        return () => { unlisten.then(f => f()); };
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
            // Fetch directory first so we can use it for the Ollama model listing
            const dir = await invoke<string>("get_ollama_directory");
            setOllamaDir(dir);

            const [local, ollama, assigns, snap] = await Promise.all([
                invoke<LocalModel[]>("list_local_models"),
                invoke<OllamaModel[]>("list_ollama_models", { manifestsDir: dir }),
                invoke<Record<string, string>>("get_agent_model_assignments"),
                invoke<DebugSnapshot>("get_debug_snapshot"),
            ]);
            setLocalModels(local);
            setOllamaModels(ollama);
            setAssignments(assigns);
            setSnapshot(snap);
        } catch (err) {
            console.error("Failed to load initial data:", err);
        }
    };

    const refreshLocalModels = async () => {
        try {
            const local = await invoke<LocalModel[]>("list_local_models");
            setLocalModels(local);
        } catch (err) {
            console.error("Failed to refresh local models:", err);
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
        setExtractError(null);
        try {
            const extractName = `${model.name}-${model.tag}`;
            await invoke("extract_ollama_model", { blobDigest: model.blob_digest, modelName: extractName });
            setExtracting(null);
            refreshData();
        } catch (err) {
            console.error("Failed to extract model:", err);
            setExtractError(String(err));
            setExtracting(null);
        }
    };

    const handleChangeOllamaDir = async () => {
        const result = await invoke<string | null>("select_ollama_directory");
        if (result) {
            setOllamaDir(result);
            try {
                const models = await invoke<OllamaModel[]>("list_ollama_models", { manifestsDir: result });
                setOllamaModels(models);
            } catch (err) {
                console.error("Failed to refresh Ollama models:", err);
            }
        }
    };

    const inf = snapshot?.inference_stats;
    const vram = snapshot?.vram;
    const vramPercent = vram ? (vram.used_mb / vram.total_mb) * 100 : 0;
    const vramClass = vramPercent > 95 ? "critical" : vramPercent > 80 ? "warning" : "good";
    
    // Find the active model's metadata from local models
    const activeModelMeta = localModels.find(m => m.is_active);

    // Build model usage segments from agent assignments for partition bar
    const MODEL_COLORS = ["#6366f1", "#22c55e", "#f59e0b", "#ef4444", "#06b6d4", "#ec4899", "#8b5cf6", "#14b8a6"];
    const modelSegments = (() => {
        const modelCounts: Record<string, number> = {};
        const totalAgents = AGENT_NAMES.length;
        for (const agent of AGENT_NAMES) {
            const assignedModel = assignments[agent] || "auto";
            modelCounts[assignedModel] = (modelCounts[assignedModel] || 0) + 1;
        }
        const uniqueModels = Object.keys(modelCounts);
        return uniqueModels.map((modelId, idx) => {
            const count = modelCounts[modelId] ?? 0;
            const meta = localModels.find(m => m.filename === modelId);
            const label = modelId === "auto" 
                ? "Auto" 
                : meta?.display_name || modelId;
            return {
                modelId,
                label,
                percent: (count / totalAgents) * 100,
                count,
                color: MODEL_COLORS[idx % MODEL_COLORS.length],
                isActive: modelId === "auto" 
                    ? !!(inf && inf.active_model)
                    : meta?.is_active ?? false,
            };
        });
    })();
    
    // Sort models
    const sortedLocal = [...localModels].sort((a, b) => {
        if (a.is_active === b.is_active) return a.display_name.localeCompare(b.display_name);
        return a.is_active ? -1 : 1;
    });

    // Filter models
    const filteredLocal = sortedLocal.filter(m => {
        if (!localSearch) return true;
        const q = localSearch.toLowerCase();
        return m.display_name.toLowerCase().includes(q) || m.filename.toLowerCase().includes(q);
    });

    const filteredOllama = ollamaModels.filter(m => {
        if (!ollamaSearch) return true;
        const q = ollamaSearch.toLowerCase();
        return m.name.toLowerCase().includes(q) || m.tag.toLowerCase().includes(q);
    });

    const modelOptions = [
        { value: "auto", label: STRINGS.MODEL_AUTO },
        ...sortedLocal.map(m => ({
            value: m.filename,
            label: m.display_name,
            description: `${m.architecture} • ${m.quantization} • ${m.size_gb.toFixed(1)}GB`
        }))
    ];

    return (
        <div
            className={`model-panel panel-glass ${panelClass}`}
            style={{
                background: "var(--bg-panel)",
                borderColor: "var(--border)",
                borderRadius: "var(--radius)",
            }}
        >
            <TitleBar
                icon={<IconModel size={14} />}
                title={STRINGS.PANEL_MODEL}
                pinned={pinned}
                onPinToggle={() => setPinned(!pinned)}
                collapsed={collapsed}
                onCollapseToggle={() => setCollapsed(c => !c)}
            />
            
            {!collapsed && (
            <div className="model-panel-body">
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
                            {/* Agent model partition bar */}
                            {modelSegments.length > 1 && (
                                <div className="partition-container">
                                    <div className="partition-label">Agent Distribution</div>
                                    <div className="partition-bar">
                                        {modelSegments.map(seg => (
                                            <div
                                                key={seg.modelId}
                                                className={`partition-segment ${seg.isActive ? "active" : ""}`}
                                                style={{ width: `${seg.percent}%`, background: seg.color }}
                                                title={`${seg.label}: ${seg.count} agent${seg.count > 1 ? "s" : ""}`}
                                            />
                                        ))}
                                    </div>
                                    <div className="partition-legend">
                                        {modelSegments.map(seg => (
                                            <div key={seg.modelId} className="legend-item">
                                                <span className="legend-swatch" style={{ background: seg.color }} />
                                                <span className="legend-text" title={seg.label}>
                                                    {seg.label} ({seg.count})
                                                </span>
                                            </div>
                                        ))}
                                    </div>
                                </div>
                            )}
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
                            <Dropdown
                                options={modelOptions}
                                value={assignments[agent] || "auto"}
                                onChange={(val) => handleAssign(agent, val)}
                                width="160px"
                            />
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

                        {activeTab === 'local' && (
                            <div className="library-toolbar">
                                <button
                                    type="button"
                                    className="action-btn open-folder-btn"
                                    onClick={() => invoke("open_models_folder").catch(err => console.error("Open folder failed:", err))}
                                >
                                    <IconFolder size={14} />
                                    <span>{STRINGS.MODEL_OPEN_FOLDER}</span>
                                </button>
                            </div>
                        )}

                        {activeTab === 'ollama' && (
                            <div className="ollama-dir-row">
                                <span className="ollama-dir-path" title={ollamaDir}>
                                    {ollamaDir ? truncatePath(ollamaDir, 40) : STRINGS.MODEL_OLLAMA_NOT_DETECTED}
                                </span>
                                <button
                                    type="button"
                                    className="action-btn"
                                    onClick={handleChangeOllamaDir}
                                >
                                    {STRINGS.MODEL_OLLAMA_CHANGE}
                                </button>
                            </div>
                        )}

                        <div className="library-search">
                            <input
                                type="text"
                                className="library-search-input"
                                placeholder={activeTab === 'local' ? STRINGS.MODEL_SEARCH_LOCAL : STRINGS.MODEL_SEARCH_OLLAMA}
                                value={activeTab === 'local' ? localSearch : ollamaSearch}
                                onChange={(e) => activeTab === 'local' ? setLocalSearch(e.target.value) : setOllamaSearch(e.target.value)}
                            />
                            {(activeTab === 'local' ? localSearch : ollamaSearch) && (
                                <button
                                    type="button"
                                    className="library-search-clear"
                                    onClick={() => activeTab === 'local' ? setLocalSearch("") : setOllamaSearch("")}
                                >
                                    ×
                                </button>
                            )}
                        </div>

                        <div className="model-list">
                            {activeTab === 'local' ? (
                                filteredLocal.map(m => (
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
                                filteredOllama.map(m => (
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
                                                <>
                                                <button 
                                                    type="button"
                                                    className="action-btn primary"
                                                    disabled={extracting === m.blob_digest}
                                                    onClick={() => handleExtract(m)}
                                                >
                                                    {extracting === m.blob_digest ? STRINGS.MODEL_EXTRACTING : STRINGS.MODEL_EXTRACT}
                                                </button>
                                                {extractError && extracting === null && (
                                                    <span className="extract-error" title={extractError}>Failed</span>
                                                )}
                                                </>
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
            )}
        </div>
    );
}
