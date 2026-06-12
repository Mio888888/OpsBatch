import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { relaunch } from '@tauri-apps/plugin-process';
import { check as checkTauriUpdate } from '@tauri-apps/plugin-updater';
import type { DownloadEvent } from '@tauri-apps/plugin-updater';
import { message } from '../components/ui';
import { useTranslation } from '../i18n';
import type { AppUpdateInfo, UpdateInstallState } from '../components/asset/constants';
import { formatUpdateBytes } from '../components/asset/constants';

export function useUpdate() {
  const { t, tText } = useTranslation();

  const [updateInfo, setUpdateInfo] = useState<AppUpdateInfo | null>(null);
  const [updateModalOpen, setUpdateModalOpen] = useState(false);
  const [updateInstallState, setUpdateInstallState] = useState<UpdateInstallState>({
    phase: 'idle',
    downloaded: 0,
  });
  const updateCheckInFlightRef = useRef(false);
  const startupUpdateRequestedRef = useRef(false);

  const checkForUpdates = useCallback(async (silent = false) => {
    if (updateCheckInFlightRef.current) return null;
    updateCheckInFlightRef.current = true;
    try {
      const nextInfo = await invoke<AppUpdateInfo>('check_app_update');
      setUpdateInfo(nextInfo);
      if (!silent) {
        if (nextInfo.hasUpdate) {
          message.success(tText('appUpdate.available', { version: nextInfo.latestVersion ?? '' }));
        } else {
          message.success(tText('appUpdate.upToDate', { version: nextInfo.currentVersion }));
        }
      }
      return nextInfo;
    } catch (error) {
      if (!silent) message.error(tText('appUpdate.checkFailed', { error: String(error) }));
      return null;
    } finally {
      updateCheckInFlightRef.current = false;
    }
  }, [tText]);

  useEffect(() => {
    if (startupUpdateRequestedRef.current) return;
    startupUpdateRequestedRef.current = true;
    void checkForUpdates(true);
  }, [checkForUpdates]);

  const handleUpdateClick = useCallback(async () => {
    setUpdateModalOpen(true);
    if (!updateInfo) void checkForUpdates(false);
  }, [checkForUpdates, updateInfo]);

  const resetUpdateInstall = useCallback(() => {
    setUpdateInstallState({ phase: 'idle', downloaded: 0 });
  }, []);

  const downloadAndInstallUpdate = useCallback(async () => {
    setUpdateInstallState({ phase: 'checking', downloaded: 0 });
    try {
      const tauriUpdate = await checkTauriUpdate();
      if (!tauriUpdate) {
        setUpdateInstallState({ phase: 'idle', downloaded: 0 });
        const latestInfo = await checkForUpdates(true);
        message.info(tText('appUpdate.noInstallableUpdate', { version: latestInfo?.currentVersion ?? updateInfo?.currentVersion ?? '' }));
        return;
      }

      let downloaded = 0;
      await tauriUpdate.downloadAndInstall((event: DownloadEvent) => {
        if (event.event === 'Started') {
          downloaded = 0;
          setUpdateInstallState({
            phase: 'downloading',
            downloaded: 0,
            total: event.data.contentLength,
          });
          return;
        }
        if (event.event === 'Progress') {
          downloaded += event.data.chunkLength;
          setUpdateInstallState((current) => ({
            ...current,
            phase: 'downloading',
            downloaded,
          }));
          return;
        }
        setUpdateInstallState((current) => ({
          ...current,
          phase: 'installing',
          downloaded,
        }));
      });
      setUpdateInstallState((current) => ({
        ...current,
        phase: 'ready',
        downloaded: current.total ?? current.downloaded,
      }));
      message.success(tText('appUpdate.installReady'));
    } catch (error) {
      setUpdateInstallState((current) => ({
        ...current,
        phase: 'error',
        error: String(error),
      }));
      message.error(tText('appUpdate.installFailed', { error: String(error) }));
    }
  }, [checkForUpdates, tText, updateInfo?.currentVersion]);

  const restartForUpdate = useCallback(async () => {
    try {
      await relaunch();
    } catch (error) {
      message.error(tText('appUpdate.restartFailed', { error: String(error) }));
    }
  }, [tText]);

  const updateBusy = updateInstallState.phase === 'checking'
    || updateInstallState.phase === 'downloading'
    || updateInstallState.phase === 'installing';
  const updatePercent = updateInstallState.total
    ? Math.min(100, Math.round((updateInstallState.downloaded / updateInstallState.total) * 100))
    : 0;
  const updateProgressLabel = updateInstallState.total
    ? `${formatUpdateBytes(updateInstallState.downloaded)} / ${formatUpdateBytes(updateInstallState.total)}`
    : formatUpdateBytes(updateInstallState.downloaded);
  const updateActionLabel = updateInstallState.phase === 'ready'
    ? t('appUpdate.restartNow')
    : updateInstallState.phase === 'checking'
      ? t('appUpdate.prepareDownload')
      : updateInstallState.phase === 'installing'
        ? t('appUpdate.installing')
        : updateInstallState.phase === 'downloading'
          ? t('appUpdate.downloading')
          : updateInstallState.phase === 'error'
            ? t('common.retry')
            : t('appUpdate.downloadAndInstall');
  const updateActionDisabled = updateBusy || !updateInfo?.hasUpdate;
  const updateAction = updateInstallState.phase === 'ready' ? restartForUpdate : downloadAndInstallUpdate;

  return {
    updateInfo,
    updateModalOpen,
    setUpdateModalOpen,
    updateInstallState,
    updateBusy,
    updatePercent,
    updateProgressLabel,
    updateActionLabel,
    updateActionDisabled,
    updateAction,
    handleUpdateClick,
    resetUpdateInstall,
  };
}
