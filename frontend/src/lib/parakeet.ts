// Types for Parakeet (NVIDIA NeMo) integration
export interface ParakeetModelInfo {
  name: string;
  path: string;
  size_mb: number;
  accuracy: ModelAccuracy;
  speed: ProcessingSpeed;
  status: ModelStatus;
  description?: string;
  quantization: QuantizationType;
}

export type QuantizationType = 'FP32' | 'Int8';
export type ModelAccuracy = 'High' | 'Good' | 'Decent';
export type ProcessingSpeed = 'Slow' | 'Medium' | 'Fast' | 'Very Fast' | 'Ultra Fast';

export type ModelStatus =
  | 'Available'
  | 'Missing'
  | { Downloading: { progress: number } }
  | { Error: string }
  | { Corrupted: { file_size: number; expected_min_size: number } };

export type CancelDownloadOutcome = 'cancelled' | 'pending';
export type ParakeetDownloadEventStatus = 'downloading' | 'completed' | 'cancelled';

export interface ParakeetDownloadProgressEvent {
  modelName: string;
  progress: number;
  downloaded_bytes?: number;
  total_bytes?: number;
  downloaded_mb?: number;
  total_mb?: number;
  speed_mbps?: number;
  status: ParakeetDownloadEventStatus;
}

export interface ParakeetEngineState {
  currentModel: string | null;
  availableModels: ParakeetModelInfo[];
  isLoading: boolean;
  error: string | null;
}

// User-friendly model display configuration
export interface ModelDisplayInfo {
  friendlyName: string;
  icon: string;
  tagline: string;
  recommended?: boolean;
  tier: 'fastest' | 'balanced' | 'precise';
}

export const MODEL_DISPLAY_CONFIG: Record<string, ModelDisplayInfo> = {
  'sense-voice-small-int8': {
    friendlyName: 'SenseVoice 中文版',
    icon: '🀄',
    tagline: '中文专用 • 速度很快，自动加标点，也支持英文和粤语',
    recommended: true,
    tier: 'fastest'
  },
  'parakeet-tdt-0.6b-v3-int8': {
    friendlyName: 'Parakeet 极速版',
    icon: '⚡',
    tagline: '实时 • 仅支持英文和欧洲语言，不支持中文',
    tier: 'fastest'
  },
  'parakeet-tdt-0.6b-v2-int8': {
    friendlyName: 'Parakeet 精简版',
    icon: '📦',
    tagline: '实时 • 仅支持英文，体积更小',
    tier: 'balanced'
  },
  'parakeet-tdt-0.6b-v3-fp32': {
    friendlyName: 'Parakeet 精准版',
    icon: '🎯',
    tagline: '20 倍实时速度 • 准确度更高',
    tier: 'precise'
  }
};

// Model configuration for Parakeet models (matching Rust implementation)
// Supported models: parakeet-tdt-0.6b in v2 and v3 variants
// Source: https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx
export const PARAKEET_MODEL_CONFIGS: Record<string, Partial<ParakeetModelInfo>> = {
  'sense-voice-small-int8': {
    description: 'SenseVoice（阿里 FunASR），中文专用，同时支持英文、粤语、日语、韩语',
    size_mb: 229, // model.int8.onnx 228MB + tokens.txt 0.3MB
    accuracy: 'High',
    speed: 'Ultra Fast',
    quantization: 'Int8'
  },
  'parakeet-tdt-0.6b-v3-int8': {
    description: '在 M4 Max 上可实时转写，针对速度优化',
    size_mb: 670, // Actual download: 652MB encoder + 18.2MB decoder + 0.2MB extras
    accuracy: 'High',
    speed: 'Ultra Fast',
    quantization: 'Int8'
  },
  'parakeet-tdt-0.6b-v2-int8': {
    description: '25 倍实时速度，体积更小且准确度良好',
    size_mb: 661, // Actual download: 652MB encoder + 9MB decoder + 0.15MB extras
    accuracy: 'High',
    speed: 'Very Fast',
    quantization: 'Int8'
  },
  'parakeet-tdt-0.6b-v3-fp32': {
    description: '在 M4 Max 上达 20 倍实时速度，精度更高',
    size_mb: 2554, // Actual download: 2.44GB + 41.8MB encoder + 72.5MB decoder + 0.2MB extras
    accuracy: 'High',
    speed: 'Fast',
    quantization: 'FP32'
  }
};

// Helper functions
export function getModelIcon(accuracy: ModelAccuracy): string {
  switch (accuracy) {
    case 'High': return '🔥';
    case 'Good': return '⚡';
    case 'Decent': return '🚀';
    default: return '📊';
  }
}

// Get user-friendly display name for a model
export function getModelDisplayName(modelName: string): string {
  const displayInfo = MODEL_DISPLAY_CONFIG[modelName];
  return displayInfo?.friendlyName || modelName;
}

// Get model display info (icon, tagline, etc.)
export function getModelDisplayInfo(modelName: string): ModelDisplayInfo | null {
  return MODEL_DISPLAY_CONFIG[modelName] || null;
}

export function getStatusColor(status: ModelStatus): string {
  if (status === 'Available') return 'green';
  if (status === 'Missing') return 'gray';
  if (typeof status === 'object' && 'Downloading' in status) return 'blue';
  if (typeof status === 'object' && 'Error' in status) return 'red';
  return 'gray';
}

export function formatFileSize(sizeMb: number): string {
  if (sizeMb >= 1000) {
    return `${(sizeMb / 1000).toFixed(1)}GB`;
  }
  return `${sizeMb}MB`;
}

// Helper function to check if model is quantized
export function isQuantizedModel(modelName: string): boolean {
  return modelName.includes('int8');
}

// Helper function to get model performance badge
export function getModelPerformanceBadge(quantization: QuantizationType): { label: string; color: string } {
  switch (quantization) {
    case 'FP32':
      return { label: '全精度', color: 'blue' };
    case 'Int8':
      return { label: 'Int8 量化', color: 'green' };
    default:
      return { label: '标准', color: 'gray' };
  }
}

export function getRecommendedModel(systemSpecs?: { ram: number; cores: number }): string {
  // Default to Int8 quantized model (fastest)
  if (!systemSpecs) return 'parakeet-tdt-0.6b-v3-int8';

  // For any system, prefer Int8 for speed
  // FP32 can be used if user explicitly wants higher precision
  return 'parakeet-tdt-0.6b-v3-int8';
}

// Tauri command wrappers for Parakeet backend
import { invoke } from '@tauri-apps/api/core';

export class ParakeetAPI {
  static async init(): Promise<void> {
    await invoke('parakeet_init');
  }

  static async getAvailableModels(): Promise<ParakeetModelInfo[]> {
    return await invoke('parakeet_get_available_models');
  }

  static async loadModel(modelName: string): Promise<void> {
    await invoke('parakeet_load_model', { modelName });
  }

  static async getCurrentModel(): Promise<string | null> {
    return await invoke('parakeet_get_current_model');
  }

  static async isModelLoaded(): Promise<boolean> {
    return await invoke('parakeet_is_model_loaded');
  }

  static async transcribeAudio(audioData: number[]): Promise<string> {
    return await invoke('parakeet_transcribe_audio', { audioData });
  }

  static async getModelsDirectory(): Promise<string> {
    return await invoke('parakeet_get_models_directory');
  }

  static async downloadModel(modelName: string): Promise<void> {
    await invoke('parakeet_download_model', { modelName });
  }

  static async cancelDownload(modelName: string): Promise<CancelDownloadOutcome> {
    return await invoke('parakeet_cancel_download', { modelName });
  }

  static async deleteCorruptedModel(modelName: string): Promise<string> {
    return await invoke('parakeet_delete_corrupted_model', { modelName });
  }

  static async hasAvailableModels(): Promise<boolean> {
    return await invoke('parakeet_has_available_models');
  }

  static async validateModelReady(): Promise<string> {
    return await invoke('parakeet_validate_model_ready');
  }

  static async openModelsFolder(): Promise<void> {
    await invoke('open_parakeet_models_folder');
  }
}
