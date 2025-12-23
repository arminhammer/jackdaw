"""Test handlers for listen read mode tests"""

def echo_handler(event):
    """
    Simple echo handler that returns whatever it receives.
    This allows us to test what the handler receives based on the read mode.
    """
    return event