'use client';

import React, { createContext, useContext, useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { PermissionStatus, OnboardingPermissions } from '@/types/onboarding';
import { resolveOnboardingSummaryModelStatus } from '@/lib/onboarding-summary-model';
import { DEFAULT_WHISPER_MODEL } from '@/constants/modelDefaults';
import type { ParakeetDownloadProgressEvent } from '@/lib/parakeet';

/** Transcription engine picked during onboarding; 'none' means transcript import only */
export type TranscriptionProvider = 'whisper' | 'parakeet' | 'none';
/** Summary setup picked during onboarding; 'later' configures nothing now */
export type SummaryChoice = 'builtin' | 'custom-openai' | 'later';

export interface CustomOpenAIChoice {
  endpoint: string;
  model: string;
  apiKey: string;
}

interface OnboardingChoices {
  transcriptionProvider: TranscriptionProvider;
  transcriptionModel: string;
  summaryChoice: SummaryChoice;
  customOpenAI: CustomOpenAIChoice;
}

// Persisted so the choices survive an app restart before onboarding completes.
// The API key is not persisted; it is saved to the database on completion.
const ONBOARDING_CHOICES_KEY = 'onboardingChoices';

function loadOnboardingChoices(): Partial<OnboardingChoices> {
  try {
    return JSON.parse(localStorage.getItem(ONBOARDING_CHOICES_KEY) || '{}');
  } catch {
    return {};
  }
}

function saveOnboardingChoices(choices: OnboardingChoices) {
  try {
    const { customOpenAI, ...rest } = choices;
    localStorage.setItem(
      ONBOARDING_CHOICES_KEY,
      JSON.stringify({ ...rest, customOpenAI: { ...customOpenAI, apiKey: '' } })
    );
  } catch {
    // Non-critical
  }
}

interface WhisperModelInfo {
  name: string;
  size_mb: number;
  status: 'Available' | 'Missing' | { Downloading: { progress: number } } | { Error: string };
}

// Whisper model recommended for this machine (Parakeet has no Chinese support).
// Falls back to the default if the backend can't answer.
export async function getRecommendedWhisperModel(): Promise<string> {
  try {
    return await invoke<string>('whisper_get_recommended_model');
  } catch (error) {
    console.warn('[OnboardingContext] Failed to get recommended Whisper model:', error);
    return DEFAULT_WHISPER_MODEL;
  }
}

// Whether the given Whisper model is fully downloaded, plus its catalog size.
export async function getWhisperModelState(modelName: string): Promise<{ available: boolean; sizeMb: number }> {
  await invoke('whisper_init');
  const models = await invoke<WhisperModelInfo[]>('whisper_get_available_models');
  const model = models.find((m) => m.name === modelName);
  return { available: model?.status === 'Available', sizeMb: model?.size_mb ?? 0 };
}

// Whether the given Parakeet model is fully downloaded, plus its catalog size.
export async function getParakeetModelState(modelName: string): Promise<{ available: boolean; sizeMb: number }> {
  await invoke('parakeet_init');
  const models = await invoke<WhisperModelInfo[]>('parakeet_get_available_models');
  const model = models.find((m) => m.name === modelName);
  return { available: model?.status === 'Available', sizeMb: model?.size_mb ?? 0 };
}

// Download state of the chosen transcription model ('none' counts as ready)
export async function getTranscriptionModelState(
  provider: TranscriptionProvider,
  modelName: string
): Promise<{ available: boolean; sizeMb: number }> {
  if (provider === 'none') return { available: true, sizeMb: 0 };
  return provider === 'parakeet' ? getParakeetModelState(modelName) : getWhisperModelState(modelName);
}

interface OnboardingStatus {
  version: string;
  completed: boolean;
  current_step: number;
  model_status: {
    parakeet: string; // Transcription model status; key kept for saved-status compatibility
    summary: string;
    selected_summary_model?: string;
  };
  last_updated: string;
}

interface SummaryModelProgressInfo {
  percent: number;
  downloadedMb: number;
  totalMb: number;
  speedMbps: number;
}

interface TranscriptionProgressInfo {
  percent: number;
  downloadedMb: number;
  totalMb: number;
  speedMbps: number;
}

interface OnboardingContextType {
  currentStep: number;
  transcriptionProvider: TranscriptionProvider;
  transcriptionModel: string;
  summaryChoice: SummaryChoice;
  customOpenAI: CustomOpenAIChoice;
  transcriptionDownloaded: boolean;
  transcriptionProgress: number;
  transcriptionProgressInfo: TranscriptionProgressInfo;
  summaryModelDownloaded: boolean;
  summaryModelProgress: number;
  summaryModelProgressInfo: SummaryModelProgressInfo;
  selectedSummaryModel: string;
  recommendedSummaryModel: string;
  databaseExists: boolean;
  isBackgroundDownloading: boolean;
  // Permissions
  permissions: OnboardingPermissions;
  permissionsSkipped: boolean;
  // Navigation
  goToStep: (step: number) => void;
  goNext: () => void;
  goPrevious: () => void;
  // Setters
  setTranscriptionChoice: (provider: TranscriptionProvider, model: string) => void;
  setSummaryChoice: (choice: SummaryChoice) => void;
  setCustomOpenAI: (config: CustomOpenAIChoice) => void;
  setTranscriptionDownloaded: (value: boolean) => void;
  setSummaryModelDownloaded: (value: boolean) => void;
  setSelectedSummaryModel: (value: string) => void;
  setDatabaseExists: (value: boolean) => void;
  setPermissionStatus: (permission: keyof OnboardingPermissions, status: PermissionStatus) => void;
  setPermissionsSkipped: (skipped: boolean) => void;
  completeOnboarding: () => Promise<void>;
  startBackgroundDownloads: (options: StartBackgroundDownloadsOptions) => Promise<void>;
  retryTranscriptionDownload: () => Promise<void>;
}

interface StartBackgroundDownloadsOptions {
  includeTranscription: boolean;
  includeSummary: boolean;
  summaryModel?: string;
}

const OnboardingContext = createContext<OnboardingContextType | undefined>(undefined);

export function OnboardingProvider({ children }: { children: React.ReactNode }) {
  const [currentStep, setCurrentStep] = useState(1);
  const [completed, setCompleted] = useState(false);
  const savedChoices = useRef(typeof window !== 'undefined' ? loadOnboardingChoices() : {}).current;
  const [transcriptionProvider, setTranscriptionProvider] = useState<TranscriptionProvider>(
    savedChoices.transcriptionProvider ?? 'whisper'
  );
  const transcriptionProviderRef = useRef<TranscriptionProvider>(savedChoices.transcriptionProvider ?? 'whisper');
  const [transcriptionModel, setTranscriptionModel] = useState<string>(
    savedChoices.transcriptionModel ?? DEFAULT_WHISPER_MODEL
  );
  const transcriptionModelRef = useRef<string>(savedChoices.transcriptionModel ?? DEFAULT_WHISPER_MODEL);
  // Set once the user has picked a transcription model, so the recommendation won't override it
  const transcriptionChosenRef = useRef<boolean>(!!savedChoices.transcriptionModel);
  const [summaryChoice, setSummaryChoiceState] = useState<SummaryChoice>(savedChoices.summaryChoice ?? 'builtin');
  const [customOpenAI, setCustomOpenAIState] = useState<CustomOpenAIChoice>(
    savedChoices.customOpenAI ?? { endpoint: '', model: '', apiKey: '' }
  );
  const [transcriptionDownloaded, setTranscriptionDownloaded] = useState(false);
  const [transcriptionProgress, setTranscriptionProgress] = useState(0);
  const [transcriptionProgressInfo, setTranscriptionProgressInfo] = useState<TranscriptionProgressInfo>({
    percent: 0,
    downloadedMb: 0,
    totalMb: 0,
    speedMbps: 0,
  });
  const [summaryModelDownloaded, setSummaryModelDownloaded] = useState(false);
  const [summaryModelProgress, setSummaryModelProgress] = useState(0);
  const [summaryModelProgressInfo, setSummaryModelProgressInfo] = useState<SummaryModelProgressInfo>({
    percent: 0,
    downloadedMb: 0,
    totalMb: 0,
    speedMbps: 0,
  });
  const [selectedSummaryModel, setSelectedSummaryModel] = useState<string>('');
  const [recommendedSummaryModel, setRecommendedSummaryModel] = useState<string>('');
  const [databaseExists, setDatabaseExists] = useState(false);
  const [isBackgroundDownloading, setIsBackgroundDownloading] = useState(false);

  // Permissions state
  const [permissions, setPermissions] = useState<OnboardingPermissions>({
    microphone: 'not_determined',
    systemAudio: 'not_determined',
    screenRecording: 'not_determined',
  });
  const [permissionsSkipped, setPermissionsSkipped] = useState(false);

  const saveTimeoutRef = useRef<NodeJS.Timeout>();

  const initializeSummaryModelSelection = async (preferredModel = selectedSummaryModel) => {
    try {
      const recommendedModel = await invoke<string>('builtin_ai_get_recommended_model');
      setRecommendedSummaryModel(recommendedModel);
      const modelToCheck = preferredModel || recommendedModel;
      setSelectedSummaryModel(modelToCheck);

      const selectedModelReady = await invoke<boolean>('builtin_ai_is_model_ready', {
        modelName: modelToCheck,
        refresh: true,
      });
      const resolved = resolveOnboardingSummaryModelStatus({
        selectedModel: preferredModel,
        recommendedModel,
        selectedModelReady,
      });

      setSelectedSummaryModel(resolved.selectedSummaryModel);
      setSummaryModelDownloaded(resolved.summaryModelDownloaded);
      console.log('[OnboardingContext] Set recommended model:', resolved.selectedSummaryModel);

      return resolved;
    } catch (error) {
      console.error('[OnboardingContext] Failed to initialize summary model:', error);
      return null;
    }
  };

  const requestSummaryModelDownload = (modelName: string) => {
    console.log('[OnboardingContext] Starting Summary Model download');
    invoke('builtin_ai_download_model', { modelName })
      .catch(err => {
        if (String(err).includes('Download already in progress')) {
          return;
        }
        console.error('[OnboardingContext] Summary Model download failed:', err);
      });
  };

  // Load status on mount and initialize database
  useEffect(() => {
    loadOnboardingStatus();
    checkDatabaseStatus();
    initializeDatabaseInBackground();
  }, []);

  // Initialize database silently in background (moved from SetupOverviewStep)
  const initializeDatabaseInBackground = async () => {
    try {
      console.log('[OnboardingContext] Starting background database initialization');
      const isFirstLaunch = await invoke<boolean>('check_first_launch');

      if (!isFirstLaunch) {
        console.log('[OnboardingContext] Database exists, skipping initialization');
        setDatabaseExists(true);
        return;
      }

      // First launch - attempt auto-detection and import
      await performAutoDetection();
    } catch (error) {
      console.error('[OnboardingContext] Database initialization failed:', error);
      // Don't throw - database init failure shouldn't block onboarding
    }
  };

  const performAutoDetection = async () => {
    // Check Homebrew (macOS only)
    if (typeof navigator !== 'undefined' && navigator.platform?.toLowerCase().includes('mac')) {
      const homebrewDbPath = '/usr/local/var/meetily/meeting_minutes.db';
      try {
        const homebrewCheck = await invoke<{ exists: boolean; size: number } | null>(
          'check_homebrew_database',
          { path: homebrewDbPath }
        );

        if (homebrewCheck?.exists) {
          console.log('[OnboardingContext] Found Homebrew database, importing');
          await invoke('import_and_initialize_database', { legacyDbPath: homebrewDbPath });
          setDatabaseExists(true);
          return;
        }
      } catch (e) {
        console.log('[OnboardingContext] Homebrew check failed, continuing:', e);
      }
    }

    // Check default legacy database location
    try {
      const legacyPath = await invoke<string | null>('check_default_legacy_database');
      if (legacyPath) {
        console.log('[OnboardingContext] Found legacy database, importing');
        await invoke('import_and_initialize_database', { legacyDbPath: legacyPath });
        setDatabaseExists(true);
        return;
      }
    } catch (e) {
      console.log('[OnboardingContext] Legacy check failed, continuing:', e);
    }

    // No legacy database found - initialize fresh
    console.log('[OnboardingContext] No legacy database found, initializing fresh');
    await invoke('initialize_fresh_database');
    setDatabaseExists(true);
  };

  const isCompletingRef = useRef(false);

  // Auto-save on state change (debounced)
  useEffect(() => {
    if (saveTimeoutRef.current) clearTimeout(saveTimeoutRef.current);

    // Don't auto-save if completed (to avoid overwriting completion status)
    // Also don't auto-save if we are currently in the process of completing
    if (completed || isCompletingRef.current) return;

    saveTimeoutRef.current = setTimeout(() => {
      saveOnboardingStatus();
    }, 1000);

    return () => {
      if (saveTimeoutRef.current) clearTimeout(saveTimeoutRef.current);
    };
  }, [currentStep, transcriptionDownloaded, summaryModelDownloaded, completed]);

  // Default to the Whisper model recommended for this machine until the user picks one
  useEffect(() => {
    getRecommendedWhisperModel().then((model) => {
      if (transcriptionChosenRef.current) return;
      transcriptionModelRef.current = model;
      setTranscriptionModel(model);
    });
  }, []);

  useEffect(() => {
    saveOnboardingChoices({ transcriptionProvider, transcriptionModel, summaryChoice, customOpenAI });
  }, [transcriptionProvider, transcriptionModel, summaryChoice, customOpenAI]);

  const setTranscriptionChoice = useCallback((provider: TranscriptionProvider, model: string) => {
    transcriptionChosenRef.current = true;
    transcriptionProviderRef.current = provider;
    transcriptionModelRef.current = model;
    setTranscriptionProvider(provider);
    setTranscriptionModel(model);
    // A new choice needs its own download check
    setTranscriptionDownloaded(false);
    setTranscriptionProgress(0);
  }, []);

  const setSummaryChoice = useCallback((choice: SummaryChoice) => setSummaryChoiceState(choice), []);
  const setCustomOpenAI = useCallback((config: CustomOpenAIChoice) => setCustomOpenAIState(config), []);

  // Listen to Whisper download progress
  useEffect(() => {
    const unlisten = listen<{ modelName: string; progress: number }>(
      'model-download-progress',
      (event) => {
        const { modelName, progress } = event.payload;
        if (modelName !== transcriptionModelRef.current) return;

        setTranscriptionProgress(progress);
        setTranscriptionProgressInfo((prev) => ({
          ...prev,
          percent: progress,
          downloadedMb: prev.totalMb * progress / 100,
        }));
        if (progress >= 100) {
          setTranscriptionDownloaded(true);
        }
      }
    );

    const unlistenComplete = listen<{ modelName: string }>(
      'model-download-complete',
      (event) => {
        const { modelName } = event.payload;
        if (modelName === transcriptionModelRef.current) {
          setTranscriptionDownloaded(true);
          setTranscriptionProgress(100);
        }
      }
    );

    const unlistenError = listen<{ modelName: string; error: string }>(
      'model-download-error',
      (event) => {
        const { modelName } = event.payload;
        if (modelName === transcriptionModelRef.current) {
          console.error('Whisper download error:', event.payload.error);
        }
      }
    );

    // Parakeet reports its own events (chosen only for English/European meetings)
    const unlistenParakeet = listen<ParakeetDownloadProgressEvent>(
      'parakeet-model-download-progress',
      (event) => {
        const { modelName, progress, downloaded_mb, total_mb, speed_mbps, status } = event.payload;
        if (modelName !== transcriptionModelRef.current) return;
        if (status === 'cancelled') {
          setTranscriptionDownloaded(false);
          setTranscriptionProgress(0);
          return;
        }
        setTranscriptionProgress(progress);
        setTranscriptionProgressInfo({
          percent: progress,
          downloadedMb: downloaded_mb ?? 0,
          totalMb: total_mb ?? 0,
          speedMbps: speed_mbps ?? 0,
        });
        if (status === 'completed') {
          setTranscriptionDownloaded(true);
        }
      }
    );

    const unlistenParakeetComplete = listen<{ modelName: string }>(
      'parakeet-model-download-complete',
      (event) => {
        if (event.payload.modelName === transcriptionModelRef.current) {
          setTranscriptionDownloaded(true);
          setTranscriptionProgress(100);
        }
      }
    );

    return () => {
      unlisten.then(fn => fn());
      unlistenComplete.then(fn => fn());
      unlistenError.then(fn => fn());
      unlistenParakeet.then(fn => fn());
      unlistenParakeetComplete.then(fn => fn());
    };
  }, []);

  // Listen to summary model (Built-in AI) download progress
  useEffect(() => {
    const unlisten = listen<{
      model: string;
      progress: number;
      downloaded_mb?: number;
      total_mb?: number;
      speed_mbps?: number;
      status: string;
    }>(
      'builtin-ai-download-progress',
      (event) => {
        const { model, progress, downloaded_mb, total_mb, speed_mbps, status } = event.payload;
        if (selectedSummaryModel && model === selectedSummaryModel) {
          setSummaryModelProgress(progress);
          setSummaryModelProgressInfo({
            percent: progress,
            downloadedMb: downloaded_mb ?? 0,
            totalMb: total_mb ?? 0,
            speedMbps: speed_mbps ?? 0,
          });
          if (status === 'completed' || progress >= 100) {
            setSummaryModelDownloaded(true);
          }
        }
      }
    );

    return () => {
      unlisten.then(fn => fn());
    };
  }, [selectedSummaryModel]);

  const checkDatabaseStatus = async () => {
    try {
      const isFirstLaunch = await invoke<boolean>('check_first_launch');
      setDatabaseExists(!isFirstLaunch);
      console.log('[OnboardingContext] Database exists:', !isFirstLaunch);
    } catch (error) {
      console.error('[OnboardingContext] Failed to check database status:', error);
      setDatabaseExists(false);
    }
  };

  const loadOnboardingStatus = async () => {
    try {
      const status = await invoke<OnboardingStatus | null>('get_onboarding_status');
      if (status) {
        console.log('[OnboardingContext] Loaded saved status:', status);

        if (status.completed) {
          setCurrentStep(status.current_step);
          setCompleted(true);
          setTranscriptionDownloaded(status.model_status.parakeet === 'downloaded');
          setSummaryModelDownloaded(status.model_status.summary === 'downloaded');
          if (status.model_status.selected_summary_model) {
            setSelectedSummaryModel(status.model_status.selected_summary_model);
          }
          console.log('[OnboardingContext] Restored completed onboarding status without model verification');
          return;
        }

        // Don't trust saved status - verify actual model status on disk
        const verifiedStatus = await verifyModelStatus(status);

        setCurrentStep(verifiedStatus.currentStep);
        setCompleted(verifiedStatus.completed);
        setTranscriptionDownloaded(verifiedStatus.transcriptionDownloaded);
        setSummaryModelDownloaded(verifiedStatus.summaryModelDownloaded);
        if (verifiedStatus.selectedSummaryModel) {
          setSelectedSummaryModel(verifiedStatus.selectedSummaryModel);
        }

        console.log('[OnboardingContext] Verified status:', verifiedStatus);

        // Check if any downloads are active to restore isBackgroundDownloading state
        await checkActiveDownloads();
      } else {
        await initializeSummaryModelSelection();
      }
    } catch (error) {
      console.error('[OnboardingContext] Failed to load onboarding status:', error);
    }
  };

  // Verify that models actually exist on disk, not just trust saved JSON
  const verifyModelStatus = async (savedStatus: OnboardingStatus) => {
    let transcriptionDownloaded = false;
    let summaryModelDownloaded = false;
    let selectedSummaryModel = '';

    // Verify the chosen transcription model exists on disk
    try {
      const provider = transcriptionProviderRef.current;
      const model = transcriptionChosenRef.current ? transcriptionModelRef.current : await getRecommendedWhisperModel();
      transcriptionDownloaded = (await getTranscriptionModelState(provider, model)).available;
      console.log('[OnboardingContext] Transcription model verified on disk:', provider, model, transcriptionDownloaded);
    } catch (error) {
      console.warn('[OnboardingContext] Failed to verify transcription model:', error);
      transcriptionDownloaded = false;
    }

    // Verify the selected/recommended Summary model exists on disk.
    try {
      const recommendedModel = await invoke<string>('builtin_ai_get_recommended_model');
      setRecommendedSummaryModel(recommendedModel);
      const savedSelectedModel = savedStatus.model_status.selected_summary_model || '';
      const modelToCheck = savedSelectedModel || recommendedModel;
      const selectedModelReady = await invoke<boolean>('builtin_ai_is_model_ready', {
        modelName: modelToCheck,
        refresh: true,
      });
      const resolved = resolveOnboardingSummaryModelStatus({
        selectedModel: savedSelectedModel,
        recommendedModel,
        selectedModelReady,
      });
      selectedSummaryModel = resolved.selectedSummaryModel;
      summaryModelDownloaded = resolved.summaryModelDownloaded;
      console.log('[OnboardingContext] Summary model verified on disk:', summaryModelDownloaded, 'model:', selectedSummaryModel);
    } catch (error) {
      console.warn('[OnboardingContext] Failed to verify Summary model:', error);
      summaryModelDownloaded = false;
    }

    // Determine the correct step based on verified status
    // New simplified flow: Step 1: Welcome, Step 2: Setup Overview, Step 3: Download Progress, Step 4: Permissions (macOS)
    let currentStep = savedStatus.current_step;
    let completed = savedStatus.completed;

    // Clamp step to new max (4)
    if (currentStep > 4) {
      currentStep = 3; // Go to download progress step
    }

    // Trust the completed status - don't revert based on model downloads
    // Downloads continue in background; user stays in main app regardless
    return {
      currentStep,
      completed,
      transcriptionDownloaded,
      summaryModelDownloaded,
      selectedSummaryModel,
    };
  };

  const saveOnboardingStatus = async () => {
    // Safety check: if we are in the process of completing, DO NOT save
    // This prevents a race condition where a download completion event triggers a save
    // that overwrites the "completed" status set by completeOnboarding
    if (isCompletingRef.current) {
      console.log('[OnboardingContext] Skipping saveOnboardingStatus because completion is in progress');
      return;
    }

    try {
      await invoke('save_onboarding_status_cmd', {
        status: {
          version: '1.0',
          completed: completed,
          current_step: currentStep,
          model_status: {
            parakeet: transcriptionDownloaded ? 'downloaded' : 'not_downloaded',
            summary: summaryModelDownloaded ? 'downloaded' : 'not_downloaded',
            selected_summary_model: selectedSummaryModel || undefined,
          },
          last_updated: new Date().toISOString(),
        },
      });
    } catch (error) {
      console.error('[OnboardingContext] Failed to save onboarding status:', error);
    }
  };

  const completeOnboarding = async () => {
    try {
      // Set completion flag to prevent race conditions with auto-save
      isCompletingRef.current = true;

      // Clear any pending auto-saves
      if (saveTimeoutRef.current) {
        clearTimeout(saveTimeoutRef.current);
        saveTimeoutRef.current = undefined;
      }

      // Built-in AI: make sure the chosen model is downloading, and save it as the summary model
      let builtinModel: string | null = null;
      if (summaryChoice === 'builtin') {
        builtinModel = selectedSummaryModel;
        if (!builtinModel) {
          builtinModel = await invoke<string>('builtin_ai_get_recommended_model');
          setSelectedSummaryModel(builtinModel);
        }

        const selectedModelReady = await invoke<boolean>('builtin_ai_is_model_ready', {
          modelName: builtinModel,
          refresh: true,
        });
        setSummaryModelDownloaded(selectedModelReady);
        if (!selectedModelReady) {
          requestSummaryModelDownload(builtinModel);
        }
      } else if (summaryChoice === 'custom-openai') {
        await invoke('api_save_custom_openai_config', {
          endpoint: customOpenAI.endpoint.trim(),
          apiKey: customOpenAI.apiKey.trim() || null,
          model: customOpenAI.model.trim(),
          maxTokens: null,
          temperature: null,
          topP: null,
        });
        await invoke('api_save_model_config', {
          provider: 'custom-openai',
          model: customOpenAI.model.trim(),
          whisperModel: 'large-v3',
          apiKey: null,
          ollamaEndpoint: null,
        });
      }

      await invoke('complete_onboarding', {
        model: builtinModel,
        transcriptionProvider: transcriptionProviderRef.current,
        transcriptionModel: transcriptionModelRef.current,
      });
      setCompleted(true);
      try {
        localStorage.removeItem(ONBOARDING_CHOICES_KEY);
      } catch {
        // Non-critical
      }
      console.log('[OnboardingContext] Onboarding completed:', {
        summary: summaryChoice,
        builtinModel,
        transcription: transcriptionProviderRef.current,
        transcriptionModel: transcriptionModelRef.current,
      });

      // Reset the flag so subsequent state updates can be saved
      isCompletingRef.current = false;
    } catch (error) {
      console.error('[OnboardingContext] Failed to complete onboarding:', error);
      isCompletingRef.current = false; // Reset flag on error
      throw error; // Re-throw so PermissionsStep can handle it
    }
  };

  // Start background downloads for models.
  const startBackgroundDownloads = async ({
    includeTranscription,
    includeSummary,
    summaryModel,
  }: StartBackgroundDownloadsOptions) => {
    console.log('[OnboardingContext] Starting background downloads:', {
      includeTranscription,
      includeSummary,
      summaryModel,
    });

    try {
      const shouldStartTranscription = includeTranscription && !transcriptionDownloaded;
      const shouldStartSummary = includeSummary && !summaryModelDownloaded && !!summaryModel;

      if (!shouldStartTranscription && !shouldStartSummary) {
        if (includeSummary && !summaryModelDownloaded && !summaryModel) {
          console.warn('[OnboardingContext] Summary Model download skipped until recommendation is loaded');
        }
        return;
      }

      setIsBackgroundDownloading(true);

      // Start the chosen transcription model first
      if (shouldStartTranscription) {
        const provider = transcriptionProviderRef.current;
        if (!transcriptionChosenRef.current && provider === 'whisper') {
          const recommended = await getRecommendedWhisperModel();
          transcriptionModelRef.current = recommended;
          setTranscriptionModel(recommended);
        }
        const model = transcriptionModelRef.current;
        const { available, sizeMb } = await getTranscriptionModelState(provider, model);
        if (available) {
          setTranscriptionDownloaded(true);
          setTranscriptionProgress(100);
        } else {
          setTranscriptionProgressInfo({ percent: 0, downloadedMb: 0, totalMb: sizeMb, speedMbps: 0 });
          console.log('[OnboardingContext] Starting transcription model download:', provider, model);
          startTranscriptionDownload(provider, model);
        }
      }

      // Start selected Summary Model download immediately so completion cannot race the request.
      if (shouldStartSummary && summaryModel) {
        requestSummaryModelDownload(summaryModel);
      }
    } catch (error) {
      console.error('[OnboardingContext] Failed to start background downloads:', error);
      setIsBackgroundDownloading(false);
      throw error;
    }
  };

  const startTranscriptionDownload = (provider: TranscriptionProvider, model: string) => {
    if (provider === 'none') return;
    const command = provider === 'parakeet' ? 'parakeet_download_model' : 'whisper_download_model';
    invoke(command, { modelName: model })
      .catch(err => console.error('[OnboardingContext] Transcription model download failed:', err));
  };

  // Check if any models are currently downloading (for re-entry)
  const checkActiveDownloads = async () => {
    try {
      const models = await invoke<any[]>('whisper_get_available_models');
      const isDownloading = models.some(m => m.status && (typeof m.status === 'object' ? 'Downloading' in m.status : m.status === 'Downloading'));
      
      if (isDownloading) {
        console.log('[OnboardingContext] Detected active background downloads on mount');
        setIsBackgroundDownloading(true);
      }
      
      // Also check for Built-in AI downloads if possible (though less critical as Whisper is the main blocker)
      
    } catch (error) {
      console.warn('[OnboardingContext] Failed to check active downloads:', error);
    }
  };

  const retryTranscriptionDownload = async () => {
    console.log('[OnboardingContext] Retrying transcription model download');
    try {
      if (transcriptionProviderRef.current === 'parakeet') {
        await invoke('parakeet_retry_download', { modelName: transcriptionModelRef.current });
      } else {
        startTranscriptionDownload(transcriptionProviderRef.current, transcriptionModelRef.current);
      }
    } catch (error) {
      console.error('[OnboardingContext] Retry failed:', error);
      throw error;
    }
  };

  const setPermissionStatus = useCallback((permission: keyof OnboardingPermissions, status: PermissionStatus) => {
    setPermissions((prev: OnboardingPermissions) => ({
      ...prev,
      [permission]: status,
    }));
  }, []);

  const goToStep = useCallback((step: number) => {
    setCurrentStep(Math.max(1, Math.min(step, 4)));
  }, []);

  const goNext = useCallback(() => {
    setCurrentStep((prev: number) => {
      const next = prev + 1;
      // Don't go past step 4
      return Math.min(next, 4);
    });
  }, []);

  const goPrevious = useCallback(() => {
    setCurrentStep((prev: number) => {
      const previous = prev - 1;
      // Don't go below step 1
      return Math.max(previous, 1);
    });
  }, []);

  return (
    <OnboardingContext.Provider
      value={{
        currentStep,
        transcriptionProvider,
        transcriptionModel,
        summaryChoice,
        customOpenAI,
        setTranscriptionChoice,
        setSummaryChoice,
        setCustomOpenAI,
        transcriptionDownloaded,
        transcriptionProgress,
        transcriptionProgressInfo,
        summaryModelDownloaded,
        summaryModelProgress,
        summaryModelProgressInfo,
        selectedSummaryModel,
        recommendedSummaryModel,
        databaseExists,
        isBackgroundDownloading,
        permissions,
        permissionsSkipped,
        goToStep,
        goNext,
        goPrevious,
        setTranscriptionDownloaded,
        setSummaryModelDownloaded,
        setSelectedSummaryModel,
        setDatabaseExists,
        setPermissionStatus,
        setPermissionsSkipped,
        completeOnboarding,
        startBackgroundDownloads,
        retryTranscriptionDownload,
      }}
    >
      {children}
    </OnboardingContext.Provider>
  );
}

export function useOnboarding() {
  const context = useContext(OnboardingContext);
  if (!context) {
    throw new Error('useOnboarding must be used within OnboardingProvider');
  }
  return context;
}
