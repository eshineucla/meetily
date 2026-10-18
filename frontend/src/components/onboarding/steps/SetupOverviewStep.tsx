import React, { useEffect, useState } from 'react';
import { CLASSIFIED_BUILD } from '@/config/features';
import { invoke } from '@tauri-apps/api/core';
import { Loader2 } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { OnboardingContainer } from '../OnboardingContainer';
import {
  useOnboarding,
  getRecommendedWhisperModel,
  type TranscriptionProvider,
  type SummaryChoice,
} from '@/contexts/OnboardingContext';
import { DEFAULT_PARAKEET_MODEL, SENSE_VOICE_MODEL } from '@/constants/modelDefaults';
import { formatSummaryModelSizeLabelFromMb } from '@/lib/onboarding-summary-model';

interface CatalogModel {
  name: string;
  size_mb: number;
  status: unknown;
}

interface BuiltinModel {
  name: string;
  display_name: string;
  size_mb: number;
  description: string;
}

// Whisper models offered at setup, with how they do on Mandarin.
// The full list stays available in Settings.
const WHISPER_CHOICES: { name: string; note: string }[] = [
  { name: 'large-v3-turbo-q5_0', note: '中文准确度和速度较平衡' },
  { name: 'medium-q5_0', note: '适合配置较低的电脑' },
  { name: 'large-v3-q5_0', note: '中文最准，但较慢' },
  { name: 'small', note: '速度快，中文准确度一般' },
];

const isAvailable = (status: unknown) => status === 'Available';

function Option({
  checked,
  onSelect,
  title,
  detail,
  badge,
}: {
  checked: boolean;
  onSelect: () => void;
  title: string;
  detail?: string;
  badge?: string;
}) {
  return (
    <label
      className={`flex items-start gap-3 rounded-md border p-3 cursor-pointer transition-colors ${
        checked ? 'border-gray-900 bg-gray-50' : 'border-gray-200 hover:bg-gray-50'
      }`}
    >
      <input type="radio" className="mt-1" checked={checked} onChange={onSelect} />
      <div className="flex-1">
        <div className="text-sm font-medium text-gray-900 flex items-center gap-2">
          {title}
          {badge && (
            <span className="text-xs font-normal px-1.5 py-0.5 rounded bg-gray-900 text-white">{badge}</span>
          )}
        </div>
        {detail && <div className="text-xs text-gray-500 mt-0.5">{detail}</div>}
      </div>
    </label>
  );
}

export function SetupOverviewStep() {
  const {
    goNext,
    transcriptionProvider,
    transcriptionModel,
    setTranscriptionChoice,
    summaryChoice,
    setSummaryChoice,
    customOpenAI,
    setCustomOpenAI,
    selectedSummaryModel,
    recommendedSummaryModel,
    setSelectedSummaryModel,
    setSummaryModelDownloaded,
  } = useOnboarding();
  const [isMac, setIsMac] = useState(false);
  const [whisperModels, setWhisperModels] = useState<CatalogModel[]>([]);
  const [parakeetModel, setParakeetModel] = useState<CatalogModel | null>(null);
  const [senseVoiceModel, setSenseVoiceModel] = useState<CatalogModel | null>(null);
  const [recommendedWhisper, setRecommendedWhisper] = useState('');
  const [builtinModels, setBuiltinModels] = useState<BuiltinModel[]>([]);
  const [isTesting, setIsTesting] = useState(false);
  const [isContinuing, setIsContinuing] = useState(false);

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

  // Load the model catalogs to choose from
  useEffect(() => {
    getRecommendedWhisperModel().then(setRecommendedWhisper);

    invoke('whisper_init')
      .then(() => invoke<CatalogModel[]>('whisper_get_available_models'))
      .then(setWhisperModels)
      .catch((error) => console.warn('[SetupOverviewStep] Failed to list Whisper models:', error));

    invoke('parakeet_init')
      .then(() => invoke<CatalogModel[]>('parakeet_get_available_models'))
      .then((models) => {
        setParakeetModel(models.find((m) => m.name === DEFAULT_PARAKEET_MODEL) ?? null);
        setSenseVoiceModel(models.find((m) => m.name === SENSE_VOICE_MODEL) ?? null);
      })
      .catch((error) => console.warn('[SetupOverviewStep] Failed to list Parakeet models:', error));

    invoke<BuiltinModel[]>('builtin_ai_list_models')
      .then(setBuiltinModels)
      .catch((error) => console.warn('[SetupOverviewStep] Failed to list built-in models:', error));
  }, []);

  const whisperOptions = WHISPER_CHOICES.map((choice) => ({
    ...choice,
    model: whisperModels.find((m) => m.name === choice.name),
  })).filter((o) => o.model);

  const sizeLabel = (model?: CatalogModel | BuiltinModel) =>
    model ? formatSummaryModelSizeLabelFromMb(model.size_mb) : '';

  const downloadedLabel = (model?: CatalogModel) => (model && isAvailable(model.status) ? '，已下载' : '');

  const customInvalid =
    summaryChoice === 'custom-openai' && (!customOpenAI.endpoint.trim() || !customOpenAI.model.trim());

  const handleTestConnection = async () => {
    setIsTesting(true);
    try {
      const result = await invoke<{ status: string; message: string }>('api_test_custom_openai_connection', {
        endpoint: customOpenAI.endpoint.trim(),
        apiKey: customOpenAI.apiKey.trim() || null,
        model: customOpenAI.model.trim(),
      });
      toast.success('连接成功', { description: result.message });
    } catch (error) {
      toast.error('连接失败', { description: String(error) });
    } finally {
      setIsTesting(false);
    }
  };

  const handleContinue = async () => {
    setIsContinuing(true);
    try {
      // The download step checks readiness of whichever built-in model was picked
      if (summaryChoice === 'builtin' && selectedSummaryModel) {
        const ready = await invoke<boolean>('builtin_ai_is_model_ready', {
          modelName: selectedSummaryModel,
          refresh: true,
        });
        setSummaryModelDownloaded(ready);
      }
    } catch (error) {
      console.warn('[SetupOverviewStep] Failed to check built-in model:', error);
    } finally {
      setIsContinuing(false);
    }
    goNext();
  };

  const pickTranscription = (provider: TranscriptionProvider, model: string) => setTranscriptionChoice(provider, model);
  const pickSummary = (choice: SummaryChoice) => setSummaryChoice(choice);

  return (
    <OnboardingContainer
      title="选择模型"
      description="选好后只下载你选中的模型。以后可以在设置里更改。"
      step={2}
      totalSteps={isMac ? 4 : 3}
    >
      <div className="flex flex-col items-center space-y-6">
        <div className="w-full max-w-lg space-y-2">
          <h3 className="font-medium text-gray-900">1. 语音转写</h3>
          {whisperOptions.length === 0 && (
            <div className="flex items-center gap-2 text-sm text-gray-500">
              <Loader2 className="w-4 h-4 animate-spin" /> 正在读取模型列表…
            </div>
          )}
          {senseVoiceModel && (
            <Option
              checked={transcriptionProvider === 'parakeet' && transcriptionModel === senseVoiceModel.name}
              onSelect={() => pickTranscription('parakeet', senseVoiceModel.name)}
              title="SenseVoice"
              detail={`${sizeLabel(senseVoiceModel)}，中文专用，速度很快，自动加标点，也支持英文和粤语${downloadedLabel(senseVoiceModel)}`}
              badge="中文专用"
            />
          )}
          {whisperOptions.map((o) => (
            <Option
              key={o.name}
              checked={transcriptionProvider === 'whisper' && transcriptionModel === o.name}
              onSelect={() => pickTranscription('whisper', o.name)}
              title={`Whisper ${o.name}`}
              detail={`${sizeLabel(o.model)}，支持中文，${o.note}${downloadedLabel(o.model)}`}
              badge={o.name === recommendedWhisper ? '推荐' : undefined}
            />
          ))}
          {parakeetModel && (
            <Option
              checked={transcriptionProvider === 'parakeet' && transcriptionModel === parakeetModel.name}
              onSelect={() => pickTranscription('parakeet', parakeetModel.name)}
              title="Parakeet"
              detail={`${sizeLabel(parakeetModel)}，仅支持英文和欧洲语言，不支持中文${downloadedLabel(parakeetModel)}`}
            />
          )}
          <Option
            checked={transcriptionProvider === 'none'}
            onSelect={() => pickTranscription('none', '')}
            title="暂不下载"
            detail="只导入现成的文字稿；以后要录音时再到设置里下载转写模型"
          />
        </div>

        <div className="w-full max-w-lg space-y-2">
          <h3 className="font-medium text-gray-900">2. 纪要生成</h3>
          {builtinModels.map((m) => (
            <Option
              key={m.name}
              checked={summaryChoice === 'builtin' && selectedSummaryModel === m.name}
              onSelect={() => {
                pickSummary('builtin');
                setSelectedSummaryModel(m.name);
              }}
              title={`内置本地模型 ${m.display_name || m.name}`}
              detail={`${sizeLabel(m)}，完全离线`}
              badge={m.name === recommendedSummaryModel ? '推荐' : undefined}
            />
          ))}
          <Option
            checked={summaryChoice === 'custom-openai'}
            onSelect={() => pickSummary('custom-openai')}
            title="自定义接口（OpenAI 兼容）"
            detail="使用自己部署的或第三方的模型服务，填写地址和密钥"
          />
          {summaryChoice === 'custom-openai' && (
            <div className="space-y-2 pl-7">
              <Input
                placeholder={CLASSIFIED_BUILD ? '内网接口地址，例如 http://192.168.1.10:8000/v1' : '接口地址，例如 http://localhost:8000/v1'}
                value={customOpenAI.endpoint}
                onChange={(e) => setCustomOpenAI({ ...customOpenAI, endpoint: e.target.value })}
              />
              <Input
                placeholder="模型名称"
                value={customOpenAI.model}
                onChange={(e) => setCustomOpenAI({ ...customOpenAI, model: e.target.value })}
              />
              <Input
                type="password"
                placeholder="API 密钥（可选）"
                value={customOpenAI.apiKey}
                onChange={(e) => setCustomOpenAI({ ...customOpenAI, apiKey: e.target.value })}
              />
              <Button variant="outline" size="sm" onClick={handleTestConnection} disabled={customInvalid || isTesting}>
                {isTesting && <Loader2 className="w-3 h-3 mr-2 animate-spin" />}
                测试连接
              </Button>
            </div>
          )}
          <Option
            checked={summaryChoice === 'later'}
            onSelect={() => pickSummary('later')}
            title="暂不设置"
            detail="以后在设置里选择 Ollama 或其他服务"
          />
        </div>

        <div className="w-full max-w-xs">
          <Button
            onClick={handleContinue}
            disabled={customInvalid || isContinuing}
            className="w-full h-11 bg-gray-900 hover:bg-gray-800 text-white"
          >
            继续
          </Button>
        </div>
      </div>
    </OnboardingContainer>
  );
}
