export function isSshAuthenticationFailure(message?: string) {
  if (!message) return false;

  return /认证被拒绝|authentication (?:failed|failure|refused)|permission denied/i.test(message);
}
