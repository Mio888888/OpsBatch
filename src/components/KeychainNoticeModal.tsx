import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Input, Modal, Space, Spin } from './ui';
import { SafetyOutlined, KeyOutlined } from './ui/icons';
import { useTranslation } from '../i18n';
import {
  KEYCHAIN_NOTICE_EVENT,
  migratePlaintextSecretsToVault,
  requestKeychainNotice,
  runStartupRepoUpdates,
  unlockLocalVault,
} from '../utils/keychainNotice';

export default function KeychainNoticeModal({ onUnlocked }: { onUnlocked?: () => void }) {
  const { t, tText } = useTranslation();
  const [open, setOpen] = useState(false);
  const [masterKey, setMasterKey] = useState('');
  const [loading, setLoading] = useState(false);
  const [pendingUnlock, setPendingUnlock] = useState(false);
  const [error, setError] = useState('');
  const resolverRef = useRef<((confirmed: boolean) => void) | null>(null);

  const close = useCallback((confirmed: boolean) => {
    setOpen(false);
    setLoading(false);
    setPendingUnlock(false);
    setError('');
    setMasterKey('');
    resolverRef.current?.(confirmed);
    resolverRef.current = null;
  }, []);

  const tryUnlock = useCallback(async (value?: string) => {
    if (loading) return;
    setLoading(true);
    setError('');
    try {
      await unlockLocalVault(value);
      await migratePlaintextSecretsToVault();
      void runStartupRepoUpdates().catch(() => {});
      close(true);
    } catch (unlockError) {
      const message = String(unlockError);
      if (message.includes('本地加密密钥不存在') || message.includes('系统钥匙串中的本地加密密钥无法解开')) {
        setPendingUnlock(true);
        setError(
          message.includes('系统钥匙串中的本地加密密钥无法解开')
            ? tText('keychainNotice.staleMasterKey')
            : tText('keychainNotice.needMasterKey'),
        );
        setLoading(false);
        return;
      }
      setError(message);
      setLoading(false);
    }
  }, [close, loading, tText]);

  useEffect(() => {
    const handleNotice = (event: Event) => {
      const resolve = (event as CustomEvent<{ resolve?: (confirmed: boolean) => void }>).detail?.resolve;
      if (!resolve) return;
      if (resolverRef.current) {
        resolve(false);
        return;
      }
      resolverRef.current = resolve;
      setOpen(true);
      setPendingUnlock(false);
      setError('');
      setMasterKey('');
      void tryUnlock();
    };

    window.addEventListener(KEYCHAIN_NOTICE_EVENT, handleNotice);
    return () => window.removeEventListener(KEYCHAIN_NOTICE_EVENT, handleNotice);
  }, [tryUnlock]);

  useEffect(() => {
    void requestKeychainNotice().then((confirmed) => {
      if (confirmed) onUnlocked?.();
    });
  }, [onUnlocked]);

  return (
    <Modal
      title={t('keychainNotice.title')}
      open={open}
      onCancel={undefined}
      onOk={undefined}
      footer={null}
      width={520}
      className="keychain-notice-modal"
      closable={false}
      maskClosable={false}
    >
      <div className="keychain-notice">
        <p>{t('keychainNotice.body')}</p>
        <div className="keychain-notice-status">
          <SafetyOutlined />
          <span>{t('keychainNotice.status')}</span>
        </div>
        {pendingUnlock ? (
          <div className="keychain-notice-form">
            <Input.Password
              autoFocus
              value={masterKey}
              placeholder={tText('keychainNotice.inputPlaceholder')}
              onChange={(event) => setMasterKey(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  void tryUnlock(masterKey);
                }
              }}
            />
            <Button
              type="primary"
              icon={<KeyOutlined />}
              loading={loading}
              onClick={() => void tryUnlock(masterKey)}
            >
              {t(loading ? 'keychainNotice.unlocking' : 'keychainNotice.unlock')}
            </Button>
          </div>
        ) : (
          <Space className="keychain-notice-hint" direction="vertical" size={6}>
            <div>{t('keychainNotice.autoUnlock')}</div>
            <div>{t('keychainNotice.lockedHint')}</div>
          </Space>
        )}
        {error ? <div className="keychain-notice-error">{error}</div> : null}
        {loading || !pendingUnlock ? (
          <div className="keychain-notice-footer">
            <Spin size="small" />
            <span>{t('keychainNotice.waiting')}</span>
          </div>
        ) : null}
      </div>
    </Modal>
  );
}
