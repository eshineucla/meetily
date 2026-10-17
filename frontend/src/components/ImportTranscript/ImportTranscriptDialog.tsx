import React, { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { FileText, Loader2 } from 'lucide-react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';
import { useSidebar } from '../Sidebar/SidebarProvider';
import { applyPinnedSummaryLanguageToMeeting } from '@/lib/summary-language-preferences';

export const TRANSCRIPT_EXTENSIONS = ['txt', 'md', 'srt', 'vtt'];

/** Window event that opens the dialog, optionally with a dropped file path */
export const OPEN_TRANSCRIPT_IMPORT_EVENT = 'open-transcript-import';

interface TranscriptFile {
  path: string;
  name: string;
  content: string;
}

interface ImportTranscriptDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** File to load when the dialog opens (drag-drop) */
  preselectedFile?: string | null;
}

export function ImportTranscriptDialog({ open, onOpenChange, preselectedFile }: ImportTranscriptDialogProps) {
  const router = useRouter();
  const { refetchMeetings } = useSidebar();
  const [title, setTitle] = useState('');
  const [text, setText] = useState('');
  const [isImporting, setIsImporting] = useState(false);

  const loadFile = useCallback((file: TranscriptFile) => {
    setText(file.content);
    setTitle((prev) => prev || file.name);
  }, []);

  // Reset on open, and load a dropped file if one was passed in
  useEffect(() => {
    if (!open) return;
    setTitle('');
    setText('');
    setIsImporting(false);
    if (preselectedFile) {
      invoke<TranscriptFile>('read_transcript_file_command', { path: preselectedFile })
        .then(loadFile)
        .catch((error) => toast.error('读取文件失败', { description: String(error) }));
    }
  }, [open, preselectedFile, loadFile]);

  const handleSelectFile = async () => {
    try {
      const file = await invoke<TranscriptFile | null>('select_transcript_file_command');
      if (file) loadFile(file);
    } catch (error) {
      toast.error('读取文件失败', { description: String(error) });
    }
  };

  const handleImport = async () => {
    setIsImporting(true);
    try {
      const meetingId = await invoke<string>('import_transcript_text_command', { title, text });
      try {
        await applyPinnedSummaryLanguageToMeeting(meetingId);
      } catch (error) {
        console.warn('Failed to apply pinned summary language to imported transcript:', error);
      }
      toast.success('文字稿已导入');
      refetchMeetings();
      onOpenChange(false);
      router.push(`/meeting-details?id=${meetingId}`);
    } catch (error) {
      toast.error('导入失败', { description: String(error) });
      setIsImporting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !isImporting && onOpenChange(next)}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <FileText className="h-5 w-5" />
            导入文字稿
          </DialogTitle>
          <DialogDescription>
            粘贴已有的会议文字稿，或选择 {TRANSCRIPT_EXTENSIONS.map((e) => `.${e}`).join(' / ')} 文件。导入后可直接生成纪要。
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <Input
            placeholder="会议名称（不填则按导入时间命名）"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            disabled={isImporting}
          />
          <Textarea
            placeholder="在这里粘贴文字稿，每行一段"
            value={text}
            onChange={(e) => setText(e.target.value)}
            disabled={isImporting}
            className="h-72 resize-none font-mono text-sm"
          />
          <div className="flex items-center justify-between text-xs text-muted-foreground">
            <Button variant="outline" size="sm" onClick={handleSelectFile} disabled={isImporting}>
              选择文件…
            </Button>
            <span>{text.length.toLocaleString()} 字</span>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isImporting}>
            取消
          </Button>
          <Button onClick={handleImport} disabled={isImporting || !text.trim()}>
            {isImporting && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
            导入
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
