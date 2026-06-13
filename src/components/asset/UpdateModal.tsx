import type { ReactNode } from 'react';
import { Button, Modal } from '../ui';
import { ReloadOutlined, UpdateOutlined } from '../ui/icons';
import { useTranslation } from '../../i18n';
import type { AppUpdateInfo, UpdateInstallState } from './constants';

interface UpdateModalProps {
  updateInfo: AppUpdateInfo | null;
  updateModalOpen: boolean;
  setUpdateModalOpen: (open: boolean) => void;
  checkForUpdates: (silent?: boolean) => Promise<AppUpdateInfo | null>;
  updateCheckBusy: boolean;
  updateInstallState: UpdateInstallState;
  updateBusy: boolean;
  updatePercent: number;
  updateProgressLabel: string;
  updateActionLabel: ReactNode;
  updateActionDisabled: boolean;
  updateAction: () => void;
  resetUpdateInstall: () => void;
}

export default function UpdateModal({
  updateInfo,
  updateModalOpen,
  setUpdateModalOpen,
  checkForUpdates,
  updateCheckBusy,
  updateInstallState,
  updateBusy,
  updatePercent,
  updateProgressLabel,
  updateActionLabel,
  updateActionDisabled,
  updateAction,
  resetUpdateInstall,
}: UpdateModalProps) {
  const { t, tText } = useTranslation();

  return (
    <Modal
      className="app-update-modal"
      title={t('appUpdate.modalTitle')}
      open={updateModalOpen}
      onCancel={() => {
        if (!updateBusy) setUpdateModalOpen(false);
      }}
      width={560}
      footer={
        <>
          <Button
            onClick={() => setUpdateModalOpen(false)}
            disabled={updateBusy}
          >
            {t('common.close')}
          </Button>
          {updateInstallState.phase === 'error' ? (
            <Button onClick={resetUpdateInstall}>{t('common.cancel')}</Button>
          ) : null}
          <Button
            icon={<ReloadOutlined spin={updateCheckBusy} />}
            loading={updateCheckBusy}
            disabled={updateBusy || updateCheckBusy}
            onClick={() => {
              void checkForUpdates(false);
            }}
          >
            {t('appUpdate.check')}
          </Button>
          <Button
            type="primary"
            loading={updateBusy}
            disabled={updateActionDisabled}
            onClick={() => {
              void updateAction();
            }}
          >
            {updateActionLabel}
          </Button>
        </>
      }
    >
      <div className="app-update-panel">
        <div className={`app-update-status${updateInfo?.hasUpdate ? ' app-update-status-new' : ''}`}>
          <span className="app-update-status-icon"><UpdateOutlined /></span>
          <div>
            <div className="app-update-status-title">
              {updateInfo
                ? updateInfo.hasUpdate
                  ? t('appUpdate.available', { version: updateInfo.latestVersion ?? '' })
                  : t('appUpdate.upToDate', { version: updateInfo.currentVersion })
                : t('appUpdate.checking')}
            </div>
            <div className="app-update-status-desc">
              {updateInfo
                ? updateInfo.hasUpdate
                  ? t('appUpdate.availableDesc')
                  : t('appUpdate.upToDateDesc')
                : t('appUpdate.checkingDesc')}
            </div>
          </div>
        </div>

        <div className="app-update-version-grid">
          <div>
            <span>{t('appUpdate.currentVersion')}</span>
            <strong>{updateInfo?.currentVersion ?? '-'}</strong>
          </div>
          <div>
            <span>{t('appUpdate.latestVersion')}</span>
            <strong>{updateInfo?.latestVersion ?? '-'}</strong>
          </div>
        </div>

        {updateInfo?.hasUpdate ? (
          <div className={`app-update-download app-update-download-${updateInstallState.phase}`}>
            <div className="app-update-download-header">
              <span>
                {updateInstallState.phase === 'ready'
                  ? t('appUpdate.readyToRestart')
                  : updateInstallState.phase === 'installing'
                    ? t('appUpdate.installing')
                    : updateInstallState.phase === 'checking'
                      ? t('appUpdate.prepareDownload')
                      : updateInstallState.phase === 'error'
                        ? t('appUpdate.downloadError')
                        : t('appUpdate.downloadProgress')}
              </span>
              <strong>{updateInstallState.total ? `${updatePercent}%` : updateProgressLabel}</strong>
            </div>
            <div
              className="app-update-progress"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={updateInstallState.total ? updatePercent : undefined}
            >
              <span style={{ width: updateInstallState.total ? `${updatePercent}%` : updateBusy ? '42%' : updateInstallState.phase === 'ready' ? '100%' : '0%' }} />
            </div>
            <div className="app-update-download-meta">
              {updateInstallState.phase === 'ready'
                ? t('appUpdate.readyDesc')
                : updateInstallState.phase === 'error'
                  ? updateInstallState.error || t('appUpdate.downloadError')
                  : updateInstallState.phase === 'idle'
                    ? t('appUpdate.inAppInstallDesc')
                    : updateProgressLabel}
            </div>
          </div>
        ) : null}

        <div className="app-update-notes">
          <div className="app-update-notes-header">
            <span>{updateInfo?.releaseTitle || t('appUpdate.releaseNotes')}</span>
            {updateInfo?.publishedAt && <small>{updateInfo.publishedAt}</small>}
          </div>
          <pre>{updateInfo?.releaseNotes?.trim() || tText('appUpdate.noReleaseNotes')}</pre>
        </div>
      </div>
    </Modal>
  );
}
