import SwiftUI
import UIKit

extension View {
    /// Swiping from the left edge goes back on this screen, also when it has
    /// a back button of its own (`navigationBarBackButtonHidden`), which
    /// otherwise turns the gesture off.
    func swipeBack() -> some View {
        background(SwipeBack().frame(width: 0, height: 0))
    }
}

/// While its screen shows, it is the navigation controller's pop-gesture
/// delegate; when the screen goes, it puts back the delegate before it.
private struct SwipeBack: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> Controller { Controller() }

    func updateUIViewController(_ controller: Controller, context: Context) {}

    final class Controller: UIViewController, UIGestureRecognizerDelegate {
        private weak var navigation: UINavigationController?
        private weak var previous: UIGestureRecognizerDelegate?

        // Appearing, not moving into the parent: a cancelled swipe, or a
        // room opened over this one and closed again, shows it once more.
        override func viewWillAppear(_ animated: Bool) {
            super.viewWillAppear(animated)
            navigation = navigationController
            guard let gesture = navigation?.interactivePopGestureRecognizer,
                  gesture.delegate !== self else { return }
            previous = gesture.delegate
            gesture.delegate = self
        }

        override func viewWillDisappear(_ animated: Bool) {
            super.viewWillDisappear(animated)
            guard let gesture = navigation?.interactivePopGestureRecognizer,
                  gesture.delegate === self else { return }
            gesture.delegate = previous
        }

        /// Not on the first screen, where there is nothing to go back to.
        func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
            (navigation?.viewControllers.count ?? 0) > 1
        }
    }
}
