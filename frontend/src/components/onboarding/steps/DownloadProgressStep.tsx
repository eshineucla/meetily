import React, { useEffect, useState, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Mic, Sparkles, Check, Loader2, Download } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding, getTranscriptionModelState } from '@/contexts/OnboardingContext';
import type { ParakeetDownloadProgressEvent } from '@/lib/parakeet';
import { toast } from 'sonner';
import { motion, AnimatePresence } from 'framer-motion';
import { getSummaryModelSizeLabel, getSummaryModelSizeMb } from '@/lib/onboarding-summary-model';

type DownloadStatus = 'waiting' | 'downloading' | 'completed' | 'cancelled' | 'error';

interface DownloadState {
  status: DownloadStatus;
  progress: number;
  downloadedMb: number;
  totalMb: number;
  speedMbps: number;
  error?: string;
}

export function DownloadProgressStep() {
  const {
    goNext,
    transcriptionProvider,
    transcriptionModel,
    summaryChoice,
    retryTranscriptionDownload,
    selectedSummaryModel,
    recommendedSummaryModel,
    transcriptionDownloaded,
    setTranscriptionDownloaded,
    summaryModelDownloaded,
    setSummaryModelDownloaded,
    startBackgroundDownloads,
    completeOnboarding,
  } = useOnboarding();

  const [isMac, setIsMac] = useState(false);

  const [transcriptionState, setTranscriptionState] = useState<DownloadState>({
    status: transcriptionDownloaded ? 'completed' : 'waiting',
    progress: transcriptionDownloaded ? 100 : 0,
    downloadedMb: 0,
    totalMb: 0,
    speedMbps: 0,
  });

  const [summaryState, setSummaryState] = useState<DownloadState>({
    status: summaryModelDownloaded ? 'completed' : 'waiting',
    progress: summaryModelDownloaded ? 100 : 0,
    downloadedMb: 0,
    totalMb: 0,
    speedMbps: 0,
  });

  const [isCompleting, setIsCompleting] = useState(false);
  const transcriptionDownloadStartedRef = useRef(false);
  const summaryDownloadStartedRef = useRef(false);
  const retryingRef = useRef(false);
  const retryingSummaryRef = useRef(false);
  const transcriptionModelRef = useRef(transcriptionModel);
  transcriptionModelRef.current = transcriptionModel;

  const needsTranscriptionDownload = transcriptionProvider !== 'none';
  const needsSummaryDownload = summaryChoice === 'builtin';

  // Show the chosen model's catalog size
  useEffect(() => {
    getTranscriptionModelState(transcriptionProvider, transcriptionModel)
      .then(({ sizeMb }) => {
        if (sizeMb > 0) setTranscriptionState((prev) => ({ ...prev, totalMb: sizeMb }));
      })
      .catch((error) => console.warn('[DownloadProgressStep] Failed to read transcription model size:', error));
  }, [transcriptionProvider, transcriptionModel]);

  // The context marks the model ready when it was already on disk (or none was chosen)
  useEffect(() => {
    if (transcriptionDownloaded) {
      setTranscriptionState((prev) => ({ ...prev, status: 'completed', progress: 100 }));
    }
  }, [transcriptionDownloaded]);

  // Retry download handler
  const handleRetryDownload = async () => {
    // Prevent multiple simultaneous retries
    if (retryingRef.current) {
      console.log('[DownloadProgressStep] Retry already in progress, ignoring');
      return;
    }

    console.log('[DownloadProgressStep] Retrying transcription model download');
    retryingRef.current = true;

    // Reset error state
    setTranscriptionState((prev) => ({
      ...prev,
      status: 'waiting',
      error: undefined,
      progress: 0,
      downloadedMb: 0,
      speedMbps: 0,
    }));

    try {
      // Progress and error events update state
      await retryTranscriptionDownload();
    } catch (error) {
      console.error('[DownloadProgressStep] Retry failed:', error);
      setTranscriptionState((prev) => ({
        ...prev,
        status: 'error',
        error: error instanceof Error ? error.message : 'Retry failed',
      }));

      toast.error('Download retry failed', {
        description: 'Please check your connection and try again.',
      });
    } finally {
      // Allow retry again after 2 seconds
      setTimeout(() => {
        retryingRef.current = false;
      }, 2000);
    }
  };

  // Retry summary download handler
  const handleRetrySummaryDownload = async () => {
    // Prevent multiple simultaneous retries
    if (retryingSummaryRef.current) {
      console.log('[DownloadProgressStep] Summary retry already in progress, ignoring');
      return;
    }

    console.log('[DownloadProgressStep] Retrying summary model download');
    retryingSummaryRef.current = true;

    // Reset error state
    setSummaryState((prev) => ({
      ...prev,
      status: 'downloading',
      error: undefined,
      progress: 0,
      downloadedMb: 0,
      totalMb: getSummaryModelSizeMb(selectedSummaryModel || recommendedSummaryModel),
      speedMbps: 0,
    }));

    try {
      // Call download command directly (no retry command exists for built-in AI)
      const modelName = selectedSummaryModel;
      if (!modelName) {
        throw new Error('Summary model recommendation is not ready yet');
      }
      await invoke('builtin_ai_download_model', { modelName });
    } catch (error) {
      console.error('[DownloadProgressStep] Summary retry failed:', error);
      setSummaryState((prev) => ({
        ...prev,
        status: 'error',
        error: error instanceof Error ? error.message : 'Retry failed',
      }));

      toast.error('Summary model download retry failed', {
        description: 'Please check your connection and try again.',
      });
    } finally {
      // Allow retry again after 2 seconds
      setTimeout(() => {
        retryingSummaryRef.current = false;
      }, 2000);
    }
  };

  // Detect platform on mount
  useEffect(() => {
    const checkPlatform = async () => {
      try {
        const { platform } = await import('@tauri-apps/plugin-os');
        setIsMac(platform() === 'macos');
      } catch (e) {
        setIsMac(navigator.userAgent.includes('Mac'));
      }
    };

    checkPlatform();
  }, []);

  // Start the required transcription model immediately; summary readiness must not block it.
  useEffect(() => {
    if (transcriptionDownloadStartedRef.current) return;
    transcriptionDownloadStartedRef.current = true;

    if (!transcriptionDownloaded) {
      setTranscriptionState((prev) => ({ ...prev, status: 'downloading' }));
    }

    startBackgroundDownloads({
      includeTranscription: true,
      includeSummary: false,
    }).catch((error) => {
      console.error('Failed to start transcription model download:', error);
      if (!transcriptionDownloaded) {
        setTranscriptionState((prev) => ({ ...prev, status: 'error', error: String(error) }));
      }
    });
  }, []);

  // Start the selected summary model only after the backend recommendation is known.
  useEffect(() => {
    if (summaryDownloadStartedRef.current) return;
    if (!needsSummaryDownload || !selectedSummaryModel) return;
    summaryDownloadStartedRef.current = true;

    startSummaryDownload();
  }, [selectedSummaryModel]);

  // Listen to Whisper download progress
  useEffect(() => {
    const unlistenProgress = listen<{ modelName: string; progress: number }>(
      'model-download-progress',
      (event) => {
        const { modelName, progress } = event.payload;
        if (modelName !== transcriptionModelRef.current) return;

        setTranscriptionState((prev) => ({
          ...prev,
          status: progress >= 100 ? 'completed' : 'downloading',
          progress,
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
        if (event.payload.modelName === transcriptionModelRef.current) {
          setTranscriptionState((prev) => ({ ...prev, status: 'completed', progress: 100 }));
          setTranscriptionDownloaded(true);
        }
      }
    );

    const unlistenError = listen<{ modelName: string; error: string }>(
      'model-download-error',
      (event) => {
        if (event.payload.modelName === transcriptionModelRef.current) {
          setTranscriptionState((prev) => ({
            ...prev,
            status: 'error',
            error: event.payload.error,
          }));
        }
      }
    );

    return () => {
      unlistenProgress.then((fn) => fn());
      unlistenComplete.then((fn) => fn());
      unlistenError.then((fn) => fn());
    };
  }, []);

  // Listen to Parakeet download progress (only when Parakeet was chosen)
  useEffect(() => {
    const unlistenProgress = listen<ParakeetDownloadProgressEvent>(
      'parakeet-model-download-progress',
      (event) => {
        const { modelName, progress, downloaded_mb, total_mb, speed_mbps, status } = event.payload;
        if (modelName !== transcriptionModelRef.current) return;

        if (status === 'cancelled') {
          setTranscriptionState((prev) => ({ ...prev, status: 'cancelled', progress: 0, downloadedMb: 0, speedMbps: 0 }));
          setTranscriptionDownloaded(false);
          return;
        }

        setTranscriptionState((prev) => ({
          ...prev,
          status: status === 'completed' ? 'completed' : 'downloading',
          progress,
          downloadedMb: downloaded_mb ?? prev.downloadedMb,
          totalMb: total_mb ?? prev.totalMb,
          speedMbps: speed_mbps ?? prev.speedMbps,
        }));
        if (status === 'completed') {
          setTranscriptionDownloaded(true);
        }
      }
    );

    const unlistenError = listen<{ modelName: string; error: string }>(
      'parakeet-model-download-error',
      (event) => {
        if (event.payload.modelName === transcriptionModelRef.current) {
          setTranscriptionState((prev) => ({ ...prev, status: 'error', error: event.payload.error }));
        }
      }
    );

    return () => {
      unlistenProgress.then((fn) => fn());
      unlistenError.then((fn) => fn());
    };
  }, []);

  // Listen to Summary Model download progress (always downloading for builtin-ai)
  useEffect(() => {
    const unlisten = listen<{
      model: string;
      progress: number;
      downloaded_mb?: number;
      total_mb?: number;
      speed_mbps?: number;
      status: string;
      error?: string;
    }>('builtin-ai-download-progress', (event) => {
      const { model, progress, downloaded_mb, total_mb, speed_mbps, status, error } = event.payload;
      if (selectedSummaryModel && model === selectedSummaryModel) {
        setSummaryState((prev) => ({
          ...prev,
          status: status === 'completed'
            ? 'completed'
            : status === 'error'
            ? 'error'
            : 'downloading',
          progress,
          downloadedMb: downloaded_mb ?? prev.downloadedMb,
          totalMb: (total_mb ?? prev.totalMb) || getSummaryModelSizeMb(model),
          speedMbps: speed_mbps ?? prev.speedMbps,
          error: status === 'error' ? error : undefined,
        }));

        if (status === 'completed' || progress >= 100) {
          setSummaryModelDownloaded(true);
        }
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [selectedSummaryModel]);

  useEffect(() => {
    const modelForSize = selectedSummaryModel || recommendedSummaryModel;
    if (!modelForSize) return;

    setSummaryState((prev) => ({
      ...prev,
      status: summaryModelDownloaded
        ? 'completed'
        : prev.status === 'completed'
        ? 'waiting'
        : prev.status,
      progress: summaryModelDownloaded
        ? 100
        : prev.status === 'completed'
        ? 0
        : prev.progress,
      totalMb: prev.totalMb || getSummaryModelSizeMb(modelForSize),
    }));
  }, [selectedSummaryModel, recommendedSummaryModel, summaryModelDownloaded]);

  const startSummaryDownload = async () => {
    if (!summaryModelDownloaded && selectedSummaryModel) {
      try {
        setSummaryState((prev) => ({
          ...prev,
          status: 'downloading',
          totalMb: getSummaryModelSizeMb(selectedSummaryModel),
        }));
        await startBackgroundDownloads({
          includeTranscription: false,
          includeSummary: true,
          summaryModel: selectedSummaryModel,
        });
      } catch (error) {
        console.error('Failed to start summary model download:', error);
        setSummaryState((prev) => ({ ...prev, status: 'error', error: String(error) }));
      }
    }
  };

  const handleContinue = async () => {
    // Verify actual model availability (catches state drift)
    try {
      const actuallyAvailable = (await getTranscriptionModelState(transcriptionProvider, transcriptionModel)).available;

      if (actuallyAvailable && !transcriptionDownloaded) {
        console.log('[DownloadProgressStep] Model available but state not updated');
        setTranscriptionDownloaded(true);
        setTranscriptionState((prev) => ({
          ...prev,
          status: 'completed',
          progress: 100,
        }));
      } else if (
        !actuallyAvailable &&
        (transcriptionState.status === 'error' || transcriptionState.status === 'cancelled')
      ) {
        toast.error('Transcription engine required', {
          description: 'Please retry the download before continuing.',
        });
        return;
      }
    } catch (error) {
      console.warn('[DownloadProgressStep] Failed to verify model:', error);
    }

    // Check if downloads are complete for toast notification
    const downloadsComplete = transcriptionState.status === 'completed' &&
      (!needsSummaryDownload || summaryState.status === 'completed');

    // Show toast if downloads still in progress
    if (!downloadsComplete) {
      toast.info('Downloads will continue in the background', {
        description: 'You can start using the app. Recording will be available once speech recognition is ready.',
        duration: 5000,
      });
    }

    if (isMac) {
      // macOS: Go to Permissions step (will complete after permissions granted)
      goNext();
    } else {
      // Non-macOS: Complete onboarding immediately (downloads continue in background)
      setIsCompleting(true);
      try {
        await completeOnboarding();

        // Small delay to ensure state is saved before reload
        await new Promise(resolve => setTimeout(resolve, 100));

        window.location.reload();
      } catch (error) {
        console.error('Failed to complete onboarding:', error);
        toast.error('Failed to complete setup', {
          description: 'Please try again.',
        });
        setIsCompleting(false);
      }
    }
  };

  const renderDownloadCard = (
    title: string,
    icon: React.ReactNode,
    state: DownloadState,
    modelSize: string,
    sizeUnit = 'MB'
  ) => (
    <div className="bg-white rounded-xl border border-gray-200 p-5">
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-full bg-gray-100 flex items-center justify-center">
            {icon}
          </div>
          <div>
            <h3 className="font-medium text-gray-900">{title}</h3>
            <p className="text-sm text-gray-500">{modelSize}</p>
          </div>
        </div>
        <div>
          {state.status === 'waiting' && (
            <span className="text-sm text-gray-500">Waiting...</span>
          )}
          {state.status === 'downloading' && (
            <Loader2 className="w-5 h-5 text-gray-700 animate-spin" />
          )}
          {state.status === 'completed' && (
            <div className="w-6 h-6 rounded-full bg-green-100 flex items-center justify-center">
              <Check className="w-4 h-4 text-green-600" />
            </div>
          )}
          {state.status === 'error' && (
            <span className="text-sm text-red-500">Failed</span>
          )}
          {state.status === 'cancelled' && (
            <span className="text-sm text-gray-500">Cancelled</span>
          )}
        </div>
      </div>

      {/* Progress Bar */}
      {(state.status === 'downloading' || state.status === 'completed') && (
        <div className="space-y-2">
          <div className="w-full h-2 bg-gray-200 rounded-full overflow-hidden">
            <div
              className="h-full bg-gradient-to-r from-gray-700 to-gray-900 rounded-full transition-all duration-300"
              style={{ width: `${state.progress}%` }}
            />
          </div>
          <div className="flex items-center justify-between text-sm">
            <span className="text-gray-600">
              {state.downloadedMb.toFixed(1)} {sizeUnit} / {state.totalMb.toFixed(1)} {sizeUnit}
            </span>
            <div className="flex items-center gap-2">
              {state.speedMbps > 0 && (
                <span className="text-gray-500">
                  {state.speedMbps.toFixed(1)} {sizeUnit}/s
                </span>
              )}
              <span className="font-semibold text-gray-900">
                {Math.round(state.progress)}%
              </span>
            </div>
          </div>
        </div>
      )}

      {(state.status === 'error' || state.status === 'cancelled') && (
        <div className="mt-2 p-3 bg-red-50 border border-red-200 rounded-md">
          <p className="text-sm text-red-600 font-medium">
            {state.status === 'cancelled' ? 'Download cancelled' : 'Download Error'}
          </p>
          {state.error && <p className="text-xs text-red-500 mt-1">{state.error}</p>}
          {(title === 'Transcription Engine' || title === 'Summary Engine') && (
            <button
              onClick={title === 'Transcription Engine' ? handleRetryDownload : handleRetrySummaryDownload}
              className="mt-3 w-full h-9 px-4 bg-gray-900 hover:bg-gray-800 text-white text-sm font-medium rounded-md transition-colors flex items-center justify-center gap-2"
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                      d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
              </svg>
              Try Again
            </button>
          )}
        </div>
      )}
    </div>
  );

  return (
    <OnboardingContainer
      title="Getting things ready"
      description="You can start using Meetily after downloading the Transcription Engine."
      step={3}
      totalSteps={isMac ? 4 : 3}
    >
      <div className="flex flex-col items-center space-y-6">
        {/* Download Cards */}
        <div className="w-full max-w-lg space-y-4">
          {needsTranscriptionDownload && renderDownloadCard(
            'Transcription Engine',
            <Mic className="w-5 h-5 text-gray-600" />,
            transcriptionState,
            `${transcriptionProvider === 'parakeet' ? 'Parakeet' : 'Whisper'} ${transcriptionModel}` +
              (transcriptionState.totalMb > 0 ? ` · ~${transcriptionState.totalMb} MB` : '')
          )}

          {needsSummaryDownload && renderDownloadCard(
            'Summary Engine',
            <Sparkles className="w-5 h-5 text-gray-600" />,
            summaryState,
            getSummaryModelSizeLabel(selectedSummaryModel || recommendedSummaryModel),
            'MiB'
          )}

          {!needsTranscriptionDownload && !needsSummaryDownload && (
            <p className="text-sm text-gray-600 text-center">没有需要下载的模型，可以直接继续。</p>
          )}
        </div>

        {/* Info Message - Only show when the transcription model is downloaded */}
        <AnimatePresence>
          {transcriptionDownloaded && needsSummaryDownload && !summaryModelDownloaded && (
            <motion.div
              initial={{ opacity: 0, y: -10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -10 }}
              transition={{ duration: 0.3, ease: 'easeOut' }}
              className="w-full max-w-lg bg-gray-100 rounded-lg p-4 text-sm text-gray-800"
            >
              <div className="flex items-start gap-3">
                <Download className="w-5 h-5 text-gray-600 flex-shrink-0 mt-0.5" />
                <div>
                  <p className="font-medium">You can continue while this finishes</p>
                  <p className="text-gray-700 mt-1">
                    Download will continue in the background.
                  </p>
                </div>
              </div>
            </motion.div>
          )}
        </AnimatePresence>

        {/* Continue Button */}
        <div className="w-full max-w-xs">
          <Button
            onClick={handleContinue}
            disabled={!transcriptionDownloaded || isCompleting}
            className="w-full h-11 bg-gray-900 hover:bg-gray-800 text-white disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {(isCompleting || !transcriptionDownloaded) ? (
              <Loader2 className="w-4 h-4 mr-2 animate-spin" />
            ) : (
              'Continue'
            )}
          </Button>
        </div>
      </div>
    </OnboardingContainer>
  );
}
