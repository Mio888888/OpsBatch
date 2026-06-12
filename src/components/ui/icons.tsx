import {
  Activity,
  AlertTriangle,
  Archive,
  CheckCircle,
  CirclePause,
  CirclePlay,
  Clock,
  Code,
  Copy,
  CircleFadingArrowUp,
  Database,
  Download,
  Edit,
  Eye,
  File,
  FileText,
  FolderOpen,
  History,
  Import,
  LoaderCircle,
  Minus,
  Plus,
  RefreshCw,
  Save,
  Search,
  Server,
  Settings,
  ShieldCheck,
  Star,
  StepForward,
  Trash2,
  Upload,
  X,
  XCircle,
  Zap,
} from 'lucide-react';
import type { CSSProperties } from 'react';

function icon(Component: typeof Activity) {
  return function Icon({ spin, style }: { spin?: boolean; style?: CSSProperties }) {
    return <Component size={16} style={style} className={spin ? 'ui-icon-spin' : undefined} />;
  };
}

export const ApartmentOutlined = icon(Archive);
export const CheckCircleOutlined = icon(CheckCircle);
export const ClearOutlined = icon(XCircle);
export const ClockCircleOutlined = icon(Clock);
export const CloseCircleOutlined = icon(XCircle);
export const CloseOutlined = icon(X);
export const CloudServerOutlined = icon(Server);
export const CodeOutlined = icon(Code);
export const CopyOutlined = icon(Copy);
export const DatabaseOutlined = icon(Database);
export const DeleteOutlined = icon(Trash2);
export const EditOutlined = icon(Edit);
export const ExportOutlined = icon(Download);
export const EyeOutlined = icon(Eye);
export const FileOutlined = icon(File);
export const FolderOpenOutlined = icon(FolderOpen);
export const ForwardOutlined = icon(StepForward);
export const GithubOutlined = icon(Code);
export const HistoryOutlined = icon(History);
export const ImportOutlined = icon(Import);
export const LoadingOutlined = icon(LoaderCircle);
export const PauseCircleOutlined = icon(CirclePause);
export const PlayCircleOutlined = icon(CirclePlay);
export const PlusOutlined = icon(Plus);
export const ProfileOutlined = icon(FileText);
export const ReloadOutlined = icon(RefreshCw);
export const SafetyOutlined = icon(ShieldCheck);
export const SaveOutlined = icon(Save);
export const SearchOutlined = icon(Search);
export const SettingOutlined = icon(Settings);
export const StarFilled = icon(Star);
export const StarOutlined = icon(Star);
export const StopOutlined = icon(Minus);
export const SyncOutlined = icon(RefreshCw);
export const ThunderboltOutlined = icon(Zap);
export const UploadOutlined = icon(Upload);
export const UpdateOutlined = icon(CircleFadingArrowUp);
export const WarningOutlined = icon(AlertTriangle);
export const CloudServerFilled = icon(Server);
