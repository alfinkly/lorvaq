import { ApiError, AuthError } from "../api";
import { t } from "../i18n";

/**
 * Text for a failed settings request. An expired session (401) is not an error
 * to show: it sends the user back to sign-in through `onAuthLost`.
 */
export function failure(err: unknown, onAuthLost: () => void): string {
  if (err instanceof AuthError) {
    onAuthLost();
    return t.sessionExpired;
  }
  return err instanceof ApiError ? err.message : String(err);
}
