import { useState, useEffect, useMemo } from 'react';
import {
  Button, Space, Modal, Form, Input, message,
  Switch, Badge, Tag,
} from '../../components/ui';
import {
  GithubOutlined, PlusOutlined, SyncOutlined, DeleteOutlined,
  CheckCircleOutlined, DatabaseOutlined, HistoryOutlined,
} from '../../components/ui/icons';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../../i18n';
import { requestKeychainNotice } from '../../utils/keychainNotice';
import { logHandledError } from '../../utils/globalLogger';
import '../../styles/pages/repo-sync.css';

interface RepoInfo {
  id: string;
  url: string;
  branch: string;
  token?: string;
  hasToken?: boolean;
  lastPulledAt?: string;
  updateOnStartup: boolean;
  enabled: boolean;
}

interface PullResult {
  added: string[];
  updated: string[];
  deleted: string[];
  errors: string[];
}

interface RepoFormValues {
  url: string;
  branch?: string;
  token?: string;
  updateOnStartup?: boolean;
}

interface PullResultSection {
  key: keyof PullResult;
  labelKey: 'repoSync.added' | 'repoSync.updated' | 'repoSync.deletedFiles' | 'repoSync.errors';
  prefix: string;
  tone: 'added' | 'updated' | 'deleted' | 'errors';
}

const PULL_RESULT_SECTIONS: PullResultSection[] = [
  { key: 'added', labelKey: 'repoSync.added', prefix: '+', tone: 'added' },
  { key: 'updated', labelKey: 'repoSync.updated', prefix: '~', tone: 'updated' },
  { key: 'deleted', labelKey: 'repoSync.deletedFiles', prefix: '-', tone: 'deleted' },
  { key: 'errors', labelKey: 'repoSync.errors', prefix: '!', tone: 'errors' },
];

export default function GitHubPage() {
  const { t, tText, language } = useTranslation();
  const [repos, setRepos] = useState<RepoInfo[]>([]);
  const [modalOpen, setModalOpen] = useState(false);
  const [pulling, setPulling] = useState<string | null>(null);
  const [pullResult, setPullResult] = useState<PullResult | null>(null);
  const [form] = Form.useForm<RepoFormValues>();

  const pullStats = useMemo(() => {
    if (!pullResult) return null;
    return {
      added: pullResult.added.length,
      updated: pullResult.updated.length,
      deleted: pullResult.deleted.length,
      errors: pullResult.errors.length,
      total: pullResult.added.length + pullResult.updated.length + pullResult.deleted.length + pullResult.errors.length,
    };
  }, [pullResult]);

  const loadRepos = async () => {
    try {
      const list = await invoke<RepoInfo[]>('list_repos');
      setRepos(list);
    } catch {
      setRepos([]);
    }
  };

  useEffect(() => { loadRepos(); }, []);

  const handleAdd = async () => {
    try {
      const values = await form.validateFields();
      if (values.token && !(await requestKeychainNotice())) return;
      await invoke('add_repo', {
        url: values.url,
        branch: values.branch || 'main',
        token: values.token || null,
        updateOnStartup: Boolean(values.updateOnStartup),
      });
      message.success(tText('repoSync.repoAdded'));
      setModalOpen(false);
      form.resetFields();
      loadRepos();
    } catch (error) {
      void logHandledError('github.addRepo', error, 'warn');
    }
  };

  const handlePull = async (repo: RepoInfo) => {
    setPulling(repo.id);
    setPullResult(null);
    try {
      if (repo.hasToken && !(await requestKeychainNotice())) return;
      const result = await invoke<PullResult>('pull_repo', { repoId: repo.id, language });
      setPullResult(result);
      const total = result.added.length + result.updated.length;
      message.success(tText('repoSync.syncComplete', { count: total }));
      loadRepos();
    } catch (e: unknown) {
      message.error(tText('repoSync.syncFailed', { error: String(e) }));
    } finally {
      setPulling(null);
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await invoke('delete_repo', { id });
      message.success(tText('repoSync.deleted'));
      loadRepos();
    } catch (e: unknown) {
      message.error(String(e));
    }
  };

  const handleToggle = async (id: string, enabled: boolean) => {
    try {
      await invoke('toggle_repo', { id, enabled });
      loadRepos();
    } catch (e: unknown) {
      message.error(String(e));
    }
  };

  const handleUpdateOnStartupChange = async (id: string, updateOnStartup: boolean) => {
    try {
      await invoke('set_repo_update_on_startup', { id, updateOnStartup });
      loadRepos();
    } catch (e: unknown) {
      message.error(String(e));
    }
  };

  return (
    <div className="page-container repo-sync-page">
      <section className="repo-sync-hero">
        <div className="repo-sync-hero-main">
          <div className="repo-sync-hero-icon"><GithubOutlined /></div>
          <div className="repo-sync-title-block">
            <span className="repo-sync-eyebrow">{t('repoSync.sourceManagement')}</span>
            <h2>{t('repoSync.title')}</h2>
            <p>{t('repoSync.description')}</p>
          </div>
        </div>
        <div className="repo-sync-hero-actions">
          <Button type="primary" icon={<PlusOutlined />} onClick={() => setModalOpen(true)}>
            {t('repoSync.addRepo')}
          </Button>
        </div>
      </section>

      <main className="repo-sync-content">
        <div className="repo-sync-card-grid">
          {repos.map((repo) => {
            const isPulling = pulling === repo.id;
            return (
              <article key={repo.id} className={`repo-sync-card${repo.enabled ? '' : ' repo-sync-card-disabled'}`}>
                <div className="repo-sync-card-header">
                  <div className="repo-sync-card-title-row">
                    <span className="repo-sync-card-icon"><GithubOutlined /></span>
                    <div className="repo-sync-card-title-text">
                      <span className="repo-sync-card-url" title={repo.url}>{repo.url}</span>
                      <span className="repo-sync-card-branch"><HistoryOutlined /> {repo.branch || '-'}</span>
                    </div>
                  </div>
                  <Badge status={repo.enabled ? 'success' : 'default'} text={tText(repo.enabled ? 'repoSync.enabled' : 'repoSync.paused')} />
                </div>

                <div className="repo-sync-card-meta-row">
                  <span className="repo-sync-meta-label" title={repo.lastPulledAt || tText('repoSync.neverSynced')}>
                    <HistoryOutlined /> {repo.lastPulledAt || tText('repoSync.neverSynced')}
                  </span>
                  <span className="repo-sync-meta-label repo-sync-meta-next">
                    {tText(repo.updateOnStartup ? 'repoSync.startupUpdateEnabled' : 'repoSync.startupUpdateDisabled')}
                  </span>
                </div>

                <div className="repo-sync-card-footer">
                  <div className="repo-sync-toggle-row">
                    <Switch size="small" checked={repo.enabled} onChange={(value) => handleToggle(repo.id, value)} />
                    <span>{tText(repo.enabled ? 'repoSync.enabledStatus' : 'repoSync.pausedStatus')}</span>
                  </div>
                  <div className="repo-sync-toggle-row">
                    <Switch
                      size="small"
                      checked={repo.updateOnStartup}
                      disabled={!repo.enabled}
                      onChange={(value) => handleUpdateOnStartupChange(repo.id, value)}
                    />
                    <span>{tText('repoSync.updateOnStartup')}</span>
                  </div>
                  <Space size={4}>
                    <Button
                      size="small"
                      type="primary"
                      icon={<SyncOutlined spin={isPulling} />}
                      disabled={!repo.enabled || !!pulling}
                      onClick={() => handlePull(repo)}
                    >
                      {t(isPulling ? 'repoSync.syncing' : 'repoSync.sync')}
                    </Button>
                    <Button size="small" danger icon={<DeleteOutlined />} onClick={() => handleDelete(repo.id)}>
                      {t('repoSync.delete')}
                    </Button>
                  </Space>
                </div>
              </article>
            );
          })}

          {repos.length === 0 && (
            <div className="repo-sync-empty-card">
              <div className="repo-sync-empty-visual"><DatabaseOutlined /></div>
              <h3>{t('repoSync.emptyTitle')}</h3>
              <p>{t('repoSync.emptyDesc')}</p>
              <Button type="primary" icon={<PlusOutlined />} onClick={() => setModalOpen(true)}>
                {t('repoSync.addFirstRepo')}
              </Button>
            </div>
          )}
        </div>

        {pullResult && pullStats && (
          <aside className="repo-sync-result-panel">
            <div className="repo-sync-result-header">
              <div>
                <span className="repo-sync-eyebrow">{t('repoSync.latestResult')}</span>
                <h3>{t('repoSync.syncResult')}</h3>
                <p>{pullStats.total > 0 ? t('repoSync.totalRecords', { count: pullStats.total }) : t('repoSync.noChanges')}</p>
              </div>
              <Tag className={`repo-sync-result-status-tag ${pullStats.errors > 0 ? 'repo-sync-result-status-error' : 'repo-sync-result-status-success'}`}>
                {t(pullStats.errors > 0 ? 'repoSync.hasErrors' : 'repoSync.syncDone')}
              </Tag>
            </div>

            <div className="repo-sync-result-stats">
              {PULL_RESULT_SECTIONS.map((section) => (
                <div key={section.key} className={`repo-sync-result-stat repo-sync-result-stat-${section.tone}`}>
                  <span>{t(section.labelKey)}</span>
                  <strong>{pullStats[section.key]}</strong>
                </div>
              ))}
            </div>

            <div className="repo-sync-result-list">
              {PULL_RESULT_SECTIONS.flatMap((section) => (
                pullResult[section.key].map((item, index) => (
                  <div key={`${section.key}-${index}`} className={`repo-sync-result-item repo-sync-result-item-${section.tone}`}>
                    <span>{section.prefix}</span>
                    <code title={item}>{item}</code>
                  </div>
                ))
              ))}
              {pullStats.total === 0 && (
                <div className="repo-sync-result-empty">
                  <CheckCircleOutlined />
                  <span>{t('repoSync.noFileChanges')}</span>
                </div>
              )}
            </div>
          </aside>
        )}
      </main>

      <Modal className="repo-sync-modal" title={tText('repoSync.addRepoModal')} open={modalOpen} onOk={handleAdd} onCancel={() => setModalOpen(false)} destroyOnHidden>
        <Form form={form} layout="vertical" className="repo-sync-form">
          <Form.Item name="url" label={tText('repoSync.repoUrl')} rules={[{ required: true, message: tText('repoSync.repoUrlRequired') }]}>
            <Input placeholder="https://github.com/xxx/ops-library" />
          </Form.Item>
          <Form.Item name="branch" label={tText('repoSync.branch')} initialValue="main">
            <Input placeholder="main" />
          </Form.Item>
          <Form.Item name="token" label={tText('repoSync.accessToken')}>
            <Input.Password placeholder={tText('repoSync.privateRepoHint')} />
          </Form.Item>
          <Form.Item
            name="updateOnStartup"
            label={tText('repoSync.updateOnStartup')}
            valuePropName="checked"
            initialValue={false}
            extra={tText('repoSync.updateOnStartupExtra')}
          >
            <Switch />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
