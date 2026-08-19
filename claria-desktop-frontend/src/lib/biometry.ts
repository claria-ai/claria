import type { BiometryKind } from "./tauri";

/**
 * What to call the sensor on the button that invokes it.
 *
 * Shared by the lock screen and the Preferences toggle so the control that
 * turns a biometric on and the control that uses it never disagree about what
 * it is called.
 */
export function biometricLabel(kind: BiometryKind): string {
  switch (kind) {
    case "touch_id":
      return "Unlock with Touch ID";
    case "face_id":
      return "Unlock with Face ID";
    case "windows_hello":
      return "Unlock with Windows Hello";
    default:
      return "Unlock with biometrics";
  }
}
