import { KouTopBar } from '@kou/ui-kit'
import { useApp } from '../../state/store'
import { useLogout } from '../auth/AuthModal'

export function TopBar() {
  const { view, mode, authed, authStatus } = useApp()
  const logout = useLogout()
  const showAuthBtn = !!authStatus?.auth_required && authed

  return <KouTopBar view={view} mode={mode} showAuthButton={showAuthBtn} onSignOut={() => void logout()} />
}
